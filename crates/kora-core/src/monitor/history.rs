//! Sample history: an in-memory ring (24 h at 15 s) backed by a capped append-only file
//! `<server>/.kora/metrics.jsonl`, reloaded when the panel starts.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{Sample, CAP, KEEP_SECS};

pub struct History {
    ring: VecDeque<Sample>,
    path: PathBuf,
    dir: PathBuf,
    /// Lines in the file now; it is rewritten from the ring once it holds twice the cap.
    file_lines: usize,
}

pub fn file_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("metrics.jsonl")
}

impl History {
    /// Load what the file holds (the last day, at most [`CAP`] samples). A damaged line is skipped.
    pub fn load(dir: &Path, now: u64) -> Self {
        let path = file_path(dir);
        let mut ring = VecDeque::new();
        let mut lines = 0;
        if let Ok(text) = std::fs::read_to_string(&path) {
            for l in text.lines() {
                lines += 1;
                if let Ok(s) = serde_json::from_str::<Sample>(l) {
                    if s.t + KEEP_SECS >= now {
                        ring.push_back(s);
                    }
                }
            }
        }
        while ring.len() > CAP {
            ring.pop_front();
        }
        Self { ring, path, dir: dir.to_path_buf(), file_lines: lines }
    }

    pub fn push(&mut self, s: Sample) {
        self.append(&s);
        self.ring.push_back(s);
        while self.ring.len() > CAP {
            self.ring.pop_front();
        }
        if self.file_lines >= CAP * 2 {
            self.rewrite();
        }
    }

    /// Best effort: monitoring must never get in the way of the server.
    fn append(&mut self, s: &Sample) {
        if !self.dir.is_dir() {
            return;
        }
        let Ok(line) = serde_json::to_string(s) else {
            return;
        };
        let _ = std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            if writeln!(f, "{line}").is_ok() {
                self.file_lines += 1;
            }
        }
    }

    fn rewrite(&mut self) {
        let mut text = String::new();
        for s in &self.ring {
            if let Ok(l) = serde_json::to_string(s) {
                text.push_str(&l);
                text.push('\n');
            }
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        if std::fs::write(&tmp, text).is_ok() && crate::fsutil::rename_retrying(&tmp, &self.path).is_ok() {
            self.file_lines = self.ring.len();
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    pub fn latest(&self) -> Option<&Sample> {
        self.ring.back()
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Samples of the last `secs` seconds, thinned to at most `max_points` (the newest is always kept).
    pub fn range(&self, now: u64, secs: u64, max_points: usize) -> Vec<Sample> {
        let from = now.saturating_sub(secs);
        let all: Vec<&Sample> = self.ring.iter().filter(|s| s.t >= from).collect();
        let step = all.len().div_ceil(max_points.max(1)).max(1);
        let mut out: Vec<Sample> = all.iter().rev().step_by(step).map(|s| (*s).clone()).collect();
        out.reverse();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: u64) -> Sample {
        Sample { t, up: true, players: Some((t % 50) as u32), ..Sample::default() }
    }

    #[test]
    fn samples_survive_a_restart_and_old_ones_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_000_000;
        let mut h = History::load(dir.path(), now);
        h.push(s(now - KEEP_SECS - 100));
        h.push(s(now - 30));
        h.push(s(now - 15));
        let again = History::load(dir.path(), now);
        assert_eq!(again.len(), 2);
        assert_eq!(again.latest().unwrap().t, now - 15);
    }

    #[test]
    fn the_ring_and_the_file_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::load(dir.path(), 10_000_000);
        for i in 0..(CAP * 2 + 10) {
            h.push(s(10_000_000 - 1000 + i as u64 % 1000));
        }
        assert_eq!(h.len(), CAP);
        let lines = std::fs::read_to_string(file_path(dir.path())).unwrap().lines().count();
        assert!(lines <= CAP + 10, "file was rewritten: {lines} lines");
    }

    #[test]
    fn a_damaged_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".kora")).unwrap();
        std::fs::write(file_path(dir.path()), format!("{}\nnot json\n{}\n", serde_json::to_string(&s(50)).unwrap(), serde_json::to_string(&s(60)).unwrap())).unwrap();
        assert_eq!(History::load(dir.path(), 100).len(), 2);
    }

    #[test]
    fn range_filters_and_thins() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::load(dir.path(), 100_000);
        for i in 0..1000u64 {
            h.push(s(100_000 - 15 * (999 - i)));
        }
        let hour = h.range(100_000, 3600, 10_000);
        assert_eq!(hour.len(), 241);
        let thin = h.range(100_000, 86_400, 100);
        assert!(thin.len() <= 100 && thin.last().unwrap().t == 100_000, "{}", thin.len());
        assert!(thin.windows(2).all(|w| w[0].t < w[1].t));
    }
}
