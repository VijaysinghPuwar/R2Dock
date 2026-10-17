//! Parser for FFmpeg's `-progress pipe:1` output: blocks of `key=value`
//! lines, each ending with `progress=continue` or `progress=end`. This
//! format is stable across versions, unlike the human-readable log.

/// One progress report.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Progress {
    /// Seconds of output written so far.
    pub out_secs: Option<f64>,
    /// Bytes written so far.
    pub total_size: Option<u64>,
    /// Encoding speed as a multiple of real time.
    pub speed: Option<f64>,
    /// The last report: FFmpeg has finished.
    pub done: bool,
}

/// Collects lines until a block is complete.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: Progress,
}

impl ProgressParser {
    /// Feed one line. Returns a report when a block ends. Unknown keys and
    /// values such as `N/A` are ignored.
    pub fn line(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.trim();
        match key {
            // Microseconds, despite the historical `out_time_ms` name.
            "out_time_us" | "out_time_ms" => {
                if let Ok(us) = value.parse::<i64>()
                    && us >= 0
                {
                    #[allow(clippy::cast_precision_loss)]
                    let secs = us as f64 / 1e6;
                    self.current.out_secs = Some(secs);
                }
            }
            "total_size" => self.current.total_size = value.parse().ok(),
            "speed" => {
                self.current.speed = value
                    .strip_suffix('x')
                    .and_then(|v| v.trim().parse().ok())
                    .filter(|v: &f64| v.is_finite());
            }
            "progress" => {
                self.current.done = value == "end";
                return Some(std::mem::take(&mut self.current));
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(text: &str) -> Vec<Progress> {
        let mut p = ProgressParser::default();
        text.lines().filter_map(|l| p.line(l)).collect()
    }

    #[test]
    fn parses_blocks() {
        let out = feed(
            "frame=0\nbitrate=  32.0kbits/s\ntotal_size=123456\nout_time_us=61500000\n\
             out_time_ms=61500000\nout_time=00:01:01.500000\nspeed=95.3x\nprogress=continue\n\
             total_size=999\nout_time_us=120000000\nspeed= 101x\nprogress=end\n",
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].out_secs, Some(61.5));
        assert_eq!(out[0].total_size, Some(123_456));
        assert_eq!(out[0].speed, Some(95.3));
        assert!(!out[0].done);
        assert_eq!(out[1].out_secs, Some(120.0));
        assert_eq!(out[1].speed, Some(101.0));
        assert!(out[1].done);
    }

    #[test]
    fn tolerates_missing_and_odd_values() {
        let out = feed(
            "out_time_us=N/A\ntotal_size=N/A\nspeed=N/A\nprogress=continue\n\
             garbage line\nout_time_us=-9223372036854775807\nspeed=infx\nprogress=continue\n\r\n",
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], Progress::default());
        assert_eq!(out[1].out_secs, None);
        assert_eq!(out[1].speed, None);
    }

    #[test]
    fn windows_line_endings() {
        let out = feed("out_time_us=1000000\r\nprogress=end\r\n");
        assert_eq!(out[0].out_secs, Some(1.0));
        assert!(out[0].done);
    }
}
