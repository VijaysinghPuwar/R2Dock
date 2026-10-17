//! Stress tests for the CSV activity log: many threads writing hostile
//! values at once, and exporting large folders.
//!
//! The default sizes keep CI fast. Run the heavy versions with
//! `cargo test -p r2dock --test activity_stress -- --ignored`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use r2dock_core::{AppError, ErrorCode};
use r2dock_lib::activity::csv::{BOM, COLUMNS, header_line};
use r2dock_lib::activity::{ActivityLog, export};

/// RFC 4180 parser for one line, independent of the writer.
fn parse(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') if cur.is_empty() => quoted = true,
            (false, ',') => cells.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    assert!(!quoted, "unterminated quote in {line:?}");
    cells.push(cur);
    cells
}

const HOSTILE: [&str; 10] = [
    "=cmd|' /C calc'!A1",
    "+1,2",
    "-2\n-3",
    "@SUM(A1:A9)",
    "\tTabbed",
    "say \"hi\", then\r\nleave",
    "Ünïcødé 名前 🎧.mp3",
    "plain/key.txt",
    " spaced ",
    "secret=FAKESECRETVALUE123",
];

fn read_lines(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap();
        out.extend(text.lines().map(str::to_string));
    }
    out
}

fn wait_for_rows(dir: &Path, rows: usize, limit: Duration) -> Vec<String> {
    let start = Instant::now();
    loop {
        if dir.exists() {
            let lines = read_lines(dir);
            // One header per daily file; a run can cross midnight UTC.
            let data = lines.iter().filter(|l| !l.contains(&header_line())).count();
            if data >= rows || start.elapsed() > limit {
                return lines;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn concurrent_writers(threads: usize, per_thread: usize) {
    let dir = tempfile::tempdir().unwrap();
    let log_dir = dir.path().join("activity");
    // Names on, so hostile values reach the file as text.
    let log = ActivityLog::start(log_dir.clone(), true, true);
    let started = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let log: Arc<ActivityLog> = log.clone();
            std::thread::spawn(move || {
                for i in 0..per_thread {
                    let hostile = HOSTILE[(t + i) % HOSTILE.len()];
                    let act = log
                        .start_action("stress")
                        .name("bucket", "media-library")
                        .name("key", hostile)
                        .field("files", i)
                        .field("note", hostile);
                    if i % 7 == 0 {
                        let err = AppError::new(ErrorCode::Network, hostile)
                            .with_detail(format!("Timeout (0), request id t{t}-{i}"));
                        act.end::<()>(&Err(err));
                    } else {
                        act.end::<()>(&Ok(()));
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let total = threads * per_thread;
    let lines = wait_for_rows(&log_dir, total, Duration::from_secs(120));
    let elapsed = started.elapsed();

    let headers = lines.iter().filter(|l| l.ends_with(&header_line())).count();
    let files = std::fs::read_dir(&log_dir).unwrap().count();
    assert_eq!(headers, files, "exactly one header per file");
    for l in lines.iter().filter(|l| l.ends_with(&header_line())) {
        assert_eq!(l.as_str(), format!("{BOM}{}", header_line()));
    }
    let rows: Vec<_> = lines
        .iter()
        .filter(|l| !l.ends_with(&header_line()))
        .collect();
    assert_eq!(rows.len(), total, "every event is written exactly once");
    let mut errors = 0;
    for row in &rows {
        let cells = parse(row);
        assert_eq!(cells.len(), COLUMNS.len(), "{row}");
        assert_eq!(cells[3], "stress");
        assert_eq!(cells[7], "media-library");
        for c in &cells {
            assert!(
                !c.starts_with(['=', '+', '-', '@', '\t']),
                "formula cell {c:?} in {row}"
            );
        }
        if cells[4] == "error" {
            errors += 1;
            assert!(cells[13].starts_with('t'), "request id column: {row}");
        }
    }
    assert!(errors > 0);
    let all = lines.join("\n");
    assert!(!all.contains("FAKESECRETVALUE123"));
    eprintln!(
        "activity stress: {total} rows from {threads} threads written in {:.2} s",
        elapsed.as_secs_f64()
    );
}

#[test]
fn many_threads_write_valid_rows() {
    concurrent_writers(8, 1_000);
}

#[test]
#[ignore = "heavy; run with --ignored"]
fn many_threads_write_valid_rows_heavy() {
    concurrent_writers(32, 5_000);
}

fn big_export(days: usize, rows_per_day: usize) {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut expected = 0;
    for d in 0..days {
        let date = (chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
            + chrono::TimeDelta::days(i64::try_from(d).unwrap()))
        .format("%Y-%m-%d")
        .to_string();
        let mut text = format!("{BOM}{}\n", header_line());
        // Written in reverse order, so the export has to sort.
        for r in (0..rows_per_day).rev() {
            let secs = r % 86_400;
            text.push_str(&format!(
                "{date}T{:02}:{:02}:{:02}.{:03}Z,{date} 00:00:00,s,row{r},ok,1,,,,,,,,,\n",
                secs / 3600,
                (secs / 60) % 60,
                secs % 60,
                r % 1000
            ));
        }
        std::fs::write(dir.path().join(format!("activity-{date}.csv")), text).unwrap();
        expected += rows_per_day;
    }
    // Plus an old JSONL file.
    let mut jsonl = String::new();
    for r in 0..rows_per_day {
        jsonl.push_str(&format!(
            "{{\"time\":\"2025-12-31T{:02}:00:00+00:00\",\"session\":\"old\",\"action\":\"old{r}\",\"outcome\":\"ok\"}}\n",
            r % 24
        ));
    }
    std::fs::write(dir.path().join("activity-2025-12-31.jsonl"), jsonl).unwrap();
    expected += rows_per_day;

    let dest = out.path().join("all.csv");
    let started = Instant::now();
    let n = export::export_csv(dir.path(), None, &dest).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(n, expected);
    let text = std::fs::read_to_string(&dest).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), format!("{BOM}{}", header_line()));
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), expected);
    assert!(
        rows.windows(2).all(|w| w[0] <= w[1]),
        "rows are in time order"
    );
    assert!(rows[0].starts_with("2025-12-31T00:00:00.000Z"));
    eprintln!(
        "export stress: {expected} rows from {} files in {:.2} s",
        days + 1,
        elapsed.as_secs_f64()
    );
}

#[test]
fn exports_a_large_folder_in_order() {
    big_export(30, 2_000);
}

#[test]
#[ignore = "heavy; run with --ignored"]
fn exports_a_large_folder_in_order_heavy() {
    big_export(365, 5_000);
}

/// Manual check against a real activity folder (read only):
/// `R2DOCK_EXPORT_DIR=<folder> R2DOCK_EXPORT_OUT=<file.csv> cargo test -p r2dock --test activity_stress real_folder -- --ignored`
#[test]
#[ignore = "manual; needs R2DOCK_EXPORT_DIR and R2DOCK_EXPORT_OUT"]
fn real_folder_exports() {
    let dir = std::env::var("R2DOCK_EXPORT_DIR").unwrap();
    let out = std::env::var("R2DOCK_EXPORT_OUT").unwrap();
    let n = export::export_csv(Path::new(&dir), None, Path::new(&out)).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    for row in text.lines().skip(1) {
        assert_eq!(parse(row).len(), COLUMNS.len(), "{row}");
    }
    eprintln!("real folder: {n} rows");
}
