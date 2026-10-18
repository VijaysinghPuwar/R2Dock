//! Export: merge the daily CSV files (and old 0.1.x JSONL files) into one
//! CSV file, in time order. Blocking; run it on a blocking thread.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::FILE_PREFIX;
use super::csv::{self, Record, RecordError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Csv,
    Jsonl,
}

/// The UTC date and format of an activity file name such as
/// `activity-2026-10-18.csv`, or `None` for any other file. Also accepts the
/// fallback name `activity-2026-10-18-<session>.csv` used while the daily
/// file is locked. Letter case is ignored, as it is on Windows and macOS.
pub fn file_date(name: &str) -> Option<(NaiveDate, FileKind)> {
    let name = name.to_ascii_lowercase();
    let rest = name.strip_prefix(FILE_PREFIX)?;
    let (stem, kind) = if let Some(d) = rest.strip_suffix(".csv") {
        (d, FileKind::Csv)
    } else {
        (rest.strip_suffix(".jsonl")?, FileKind::Jsonl)
    };
    let (date, suffix) = match stem.get(10..) {
        Some("") => (stem, None),
        Some(tail) => (&stem[..10], Some(tail.strip_prefix('-')?)),
        None => return None,
    };
    let valid_suffix =
        |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_alphanumeric());
    if suffix.is_some_and(|s| kind != FileKind::Csv || !valid_suffix(s)) {
        return None;
    }
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some((date, kind))
}

/// A line of the 0.1.x JSONL format.
#[derive(Deserialize)]
struct OldLine {
    time: String,
    session: String,
    action: String,
    outcome: String,
    ms: Option<u64>,
    details: Option<Map<String, Value>>,
    error: Option<OldError>,
}

#[derive(Deserialize)]
struct OldError {
    code: String,
    message: String,
    detail: Option<String>,
}

fn old_line_to_csv(line: &str) -> Option<String> {
    let old: OldLine = serde_json::from_str(line).ok()?;
    let time = DateTime::parse_from_rfc3339(&old.time)
        .ok()?
        .with_timezone(&Utc);
    let record = Record {
        time,
        session: old.session,
        action: old.action,
        outcome: old.outcome,
        duration_ms: old.ms,
        fields: old.details.unwrap_or_default(),
        error: old.error.map(|e| RecordError {
            code: e.code,
            message: e.message,
            detail: e.detail,
        }),
    };
    Some(record.to_line())
}

/// Whether a CSV line is a data row (its first cell is a time), not the
/// header or anything else.
fn row_time(line: &str) -> Option<&str> {
    let first = line.split(',').next()?;
    NaiveDateTime::parse_from_str(first, "%Y-%m-%dT%H:%M:%S%.3fZ").ok()?;
    Some(first)
}

/// Call `f` with each line of a file, tolerating invalid UTF-8.
fn for_each_line(path: &Path, mut f: impl FnMut(&str)) -> io::Result<()> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\n', '\r']);
        f(line.trim_start_matches(csv::BOM));
    }
}

/// Merge every activity file in `dir` with rows at or after `since` (all
/// rows when `None`) into one CSV file at `dest`. Returns the row count.
///
/// Files are named by the UTC date of their rows, so days are processed in
/// order and only one day's rows are held in memory at a time.
pub fn export_csv(dir: &Path, since: Option<DateTime<Utc>>, dest: &Path) -> io::Result<usize> {
    let since_text = since.map(csv::format_time);
    let since_date = since.map(|s| s.date_naive());
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect::<Vec<_>>(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let mut days: BTreeMap<NaiveDate, Vec<(PathBuf, FileKind)>> = BTreeMap::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((date, kind)) = file_date(&name) else {
            continue;
        };
        // Symlinks are not followed.
        if since_date.is_some_and(|s| date < s) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        days.entry(date).or_default().push((entry.path(), kind));
    }

    let mut out = BufWriter::new(File::create(dest)?);
    writeln!(out, "{}{}", csv::BOM, csv::header_line())?;
    let keep = |row: &str| since_text.as_deref().is_none_or(|s| row >= s);
    let mut total = 0;
    for files in days.values() {
        let mut rows: Vec<String> = Vec::new();
        for (path, kind) in files {
            for_each_line(path, |line| {
                let row = match kind {
                    FileKind::Csv => row_time(line).map(|_| line.to_string()),
                    FileKind::Jsonl => old_line_to_csv(line),
                };
                if let Some(row) = row.filter(|r| keep(r)) {
                    rows.push(row);
                }
            })?;
        }
        rows.sort();
        for row in &rows {
            writeln!(out, "{row}")?;
        }
        total += rows.len();
    }
    out.flush()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn csv_file(rows: &[&str]) -> String {
        let mut s = format!("{}{}\n", csv::BOM, csv::header_line());
        for r in rows {
            s.push_str(r);
            s.push('\n');
        }
        s
    }

    fn row(time: &str, action: &str) -> String {
        format!("{time},2026-01-01 00:00:00,s1,{action},ok,1,,,,,,,,,")
    }

    fn read_rows(path: &Path) -> Vec<String> {
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap(),
            format!("{}{}", csv::BOM, csv::header_line())
        );
        lines.map(str::to_string).collect()
    }

    #[test]
    fn file_names() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        assert_eq!(
            file_date("activity-2026-10-18.csv"),
            Some((d, FileKind::Csv))
        );
        assert_eq!(
            file_date("activity-2026-10-18.jsonl"),
            Some((d, FileKind::Jsonl))
        );
        assert_eq!(file_date("activity-2026-13-18.csv"), None);
        assert_eq!(file_date("activity-x.csv"), None);
        assert_eq!(file_date("r2dock-activity-2026-10-18.csv"), None);
        assert_eq!(file_date("activity-2026-10-18.txt"), None);
        // Fallback files written while the daily file is locked.
        assert_eq!(
            file_date("activity-2026-10-18-ab12cd34.csv"),
            Some((d, FileKind::Csv))
        );
        assert_eq!(file_date("activity-2026-10-18-.csv"), None);
        assert_eq!(file_date("activity-2026-10-18-a b.csv"), None);
        assert_eq!(file_date("activity-2026-10-18-ab12cd34.jsonl"), None);
        assert_eq!(file_date("activity-2026-10-18x.csv"), None);
        // Case does not matter.
        assert_eq!(
            file_date("Activity-2026-10-18.CSV"),
            Some((d, FileKind::Csv))
        );
        assert_eq!(
            file_date("ACTIVITY-2026-10-18.JsonL"),
            Some((d, FileKind::Jsonl))
        );
    }

    #[test]
    fn merges_csv_and_jsonl_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "activity-2026-10-02.csv",
            &csv_file(&[
                &row("2026-10-02T10:00:00.000Z", "b"),
                &row("2026-10-02T08:00:00.000Z", "a2"),
            ]),
        );
        write(
            dir.path(),
            "activity-2026-10-01.jsonl",
            concat!(
                r##"{"time":"2026-10-01T09:00:00.123456+00:00","session":"old1","action":"object_list","outcome":"ok","ms":12,"details":{"bucket":"#9f8e7d6c","files":3}}"##,
                "\n",
                r##"{"time":"2026-10-01T09:30:00+00:00","session":"old1","action":"delete_execute","outcome":"error","ms":5,"error":{"code":"accessDenied","message":"This key cannot do that.","detail":"AccessDenied (403), request id r1"}}"##,
                "\n",
                "not json\n",
            ),
        );
        write(dir.path(), "notes.txt", "ignored");
        let dest = out.path().join("export.csv");
        let n = export_csv(dir.path(), None, &dest).unwrap();
        assert_eq!(n, 4);
        let rows = read_rows(&dest);
        assert!(rows[0].starts_with("2026-10-01T09:00:00.123Z,"));
        assert!(rows[0].contains(",object_list,ok,12,,#9f8e7d6c,,3,"));
        assert!(rows[1].contains(",delete_execute,error,"));
        assert!(rows[1].contains(",accessDenied,This key cannot do that.,r1,"));
        assert!(rows[2].contains(",a2,"));
        assert!(rows[3].contains(",b,"));
    }

    #[test]
    fn merges_fallback_files_into_their_day() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "activity-2026-10-02.csv",
            &csv_file(&[
                &row("2026-10-02T08:00:00.000Z", "a"),
                &row("2026-10-02T10:00:00.000Z", "c"),
            ]),
        );
        write(
            dir.path(),
            "activity-2026-10-02-ab12cd34.csv",
            &csv_file(&[&row("2026-10-02T09:00:00.000Z", "b")]),
        );
        let dest = out.path().join("export.csv");
        assert_eq!(export_csv(dir.path(), None, &dest).unwrap(), 3);
        let rows = read_rows(&dest);
        assert!(rows[0].contains(",a,"));
        assert!(rows[1].contains(",b,"));
        assert!(rows[2].contains(",c,"));
    }

    #[test]
    fn respects_the_date_range() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "activity-2026-10-01.csv",
            &csv_file(&[&row("2026-10-01T23:00:00.000Z", "old")]),
        );
        write(
            dir.path(),
            "activity-2026-10-02.csv",
            &csv_file(&[
                &row("2026-10-02T08:59:59.999Z", "before"),
                &row("2026-10-02T09:00:00.000Z", "at"),
                &row("2026-10-02T12:00:00.000Z", "after"),
            ]),
        );
        let since = Utc.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap();
        let dest = out.path().join("range.csv");
        assert_eq!(export_csv(dir.path(), Some(since), &dest).unwrap(), 2);
        let rows = read_rows(&dest);
        assert!(rows[0].contains(",at,"));
        assert!(rows[1].contains(",after,"));
    }

    #[test]
    fn empty_or_missing_folder_gives_header_only() {
        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("empty.csv");
        let missing = out.path().join("no-such-folder");
        assert_eq!(export_csv(&missing, None, &dest).unwrap(), 0);
        assert!(read_rows(&dest).is_empty());
    }

    #[test]
    fn writes_only_to_the_chosen_path() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "activity-2026-10-02.csv",
            &csv_file(&[&row("2026-10-02T09:00:00.000Z", "x")]),
        );
        let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        let dest = out.path().join("only.csv");
        export_csv(dir.path(), None, &dest).unwrap();
        let after: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(before.len(), after.len());
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
    }

    #[test]
    fn exported_rows_keep_names_hashed_and_secrets_out() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        // A 0.1.x line that somehow kept a secret is redacted on export.
        write(
            dir.path(),
            "activity-2026-10-01.jsonl",
            concat!(
                r##"{"time":"2026-10-01T09:00:00Z","session":"s","action":"x","outcome":"ok","details":{"note":"secret=FAKESECRETVALUE123","bucket":"#9f8e7d6c"}}"##,
                "\n"
            ),
        );
        let dest = out.path().join("e.csv");
        export_csv(dir.path(), None, &dest).unwrap();
        let text = std::fs::read_to_string(&dest).unwrap();
        assert!(!text.contains("FAKESECRETVALUE123"));
        assert!(text.contains("#9f8e7d6c"));
    }
}
