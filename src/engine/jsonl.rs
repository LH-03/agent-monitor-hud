//! Bounded JSONL readers: an incremental forward reader that only consumes
//! appended bytes, and a reverse tail reader for first hydration.

use crate::util::fsx;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Records larger than this are skipped (the old reader used the same bound).
pub const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct IncrementalReader {
    pub offset: u64,
    pending: Vec<u8>,
    discarding: bool,
    pub has_unread: bool,
    pub last_read_bytes: u64,
}

impl IncrementalReader {
    pub fn at(offset: u64) -> Self {
        IncrementalReader { offset, ..Default::default() }
    }

    pub fn reset(&mut self) {
        *self = IncrementalReader::default();
    }

    /// Reads at most `budget` new bytes and calls `on_line` for every complete
    /// record. A truncated file restarts from the beginning and returns `true`
    /// in the first tuple field so the caller can drop derived state.
    pub fn read_appended(
        &mut self,
        path: &Path,
        budget: u64,
        mut on_line: impl FnMut(&[u8]),
    ) -> io::Result<bool> {
        let mut file = fsx::open_shared(path)?;
        let length = file.metadata()?.len();
        let mut restarted = false;
        if length < self.offset {
            self.reset();
            restarted = true;
        }
        let available = length - self.offset;
        let to_read = available.min(budget);
        self.last_read_bytes = to_read;
        self.has_unread = available > to_read;
        if to_read == 0 {
            return Ok(restarted);
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buffer = vec![0u8; to_read.min(256 * 1024) as usize];
        let mut remaining = to_read;
        while remaining > 0 {
            let chunk = remaining.min(buffer.len() as u64) as usize;
            let read = file.read(&mut buffer[..chunk])?;
            if read == 0 {
                break;
            }
            remaining -= read as u64;
            self.offset += read as u64;
            self.feed(&buffer[..read], &mut on_line);
        }
        Ok(restarted)
    }

    fn feed(&mut self, mut data: &[u8], on_line: &mut impl FnMut(&[u8])) {
        while let Some(newline) = memchr(b'\n', data) {
            let segment = &data[..newline];
            if self.discarding {
                self.discarding = false;
            } else if self.pending.is_empty() {
                on_line(trim_cr(segment));
            } else if self.pending.len() + segment.len() <= MAX_RECORD_BYTES {
                self.pending.extend_from_slice(segment);
                let line = std::mem::take(&mut self.pending);
                on_line(trim_cr(&line));
            }
            self.pending.clear();
            data = &data[newline + 1..];
        }
        if !data.is_empty() && !self.discarding {
            if self.pending.len() + data.len() > MAX_RECORD_BYTES {
                self.pending = Vec::new();
                self.discarding = true;
            } else {
                self.pending.extend_from_slice(data);
            }
        }
        if self.pending.capacity() > 1024 * 1024 && self.pending.is_empty() {
            self.pending = Vec::new();
        }
    }
}

fn trim_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

pub fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|b| *b == needle)
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return needle.is_empty();
    }
    let first = needle[0];
    let last_start = haystack.len() - needle.len();
    let mut i = 0;
    while i <= last_start {
        match haystack[i..=last_start].iter().position(|b| *b == first) {
            Some(p) => {
                let start = i + p;
                if &haystack[start..start + needle.len()] == needle {
                    return true;
                }
                i = start + 1;
            }
            None => return false,
        }
    }
    false
}

/// Calls `on_line` for complete records from the end of the file backwards,
/// scanning at most `max_scan` bytes and `max_lines` records, until the
/// callback returns `false`. Returns the file length at the time of reading.
pub fn read_tail_reverse(
    path: &Path,
    max_scan: u64,
    max_lines: usize,
    mut on_line: impl FnMut(&[u8]) -> bool,
) -> io::Result<u64> {
    let mut file = fsx::open_shared(path)?;
    let length = file.metadata()?.len();
    let earliest = length.saturating_sub(max_scan);
    let mut position = length;
    let mut buffer = vec![0u8; 64 * 1024];
    // Bytes of the record currently being assembled, stored reversed-chunk wise.
    let mut pending: Vec<u8> = Vec::new();
    let mut discarding = false;
    let mut lines = 0usize;
    let mut emit = |line: &[u8], lines: &mut usize| -> bool {
        let line = trim_cr(line);
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            return true;
        }
        *lines += 1;
        on_line(line)
    };
    'outer: while position > earliest && lines < max_lines {
        let start = earliest.max(position.saturating_sub(buffer.len() as u64));
        let count = (position - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..count])?;
        let mut end = count;
        loop {
            let newline = buffer[..end].iter().rposition(|b| *b == b'\n');
            let segment_start = newline.map(|n| n + 1).unwrap_or(0);
            let segment = &buffer[segment_start..end];
            if !discarding {
                if pending.len() + segment.len() > MAX_RECORD_BYTES {
                    pending.clear();
                    discarding = true;
                } else if newline.is_some() {
                    if pending.is_empty() {
                        if !emit(segment, &mut lines) {
                            break 'outer;
                        }
                    } else {
                        let mut record = segment.to_vec();
                        record.extend_from_slice(&pending);
                        pending.clear();
                        if !emit(&record, &mut lines) {
                            break 'outer;
                        }
                    }
                } else {
                    let mut record = segment.to_vec();
                    record.extend_from_slice(&pending);
                    pending = record;
                }
            }
            match newline {
                Some(n) => {
                    discarding = false;
                    end = n;
                    if lines >= max_lines {
                        break 'outer;
                    }
                }
                None => break,
            }
        }
        position = start;
    }
    // At the very beginning of the file the oldest partial record is complete.
    if position == 0 && !discarding && !pending.is_empty() && lines < max_lines {
        emit(&pending, &mut lines);
    }
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn incremental_and_tail() {
        let dir = std::env::temp_dir().join(format!("hud-jsonl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jsonl");
        std::fs::write(&path, b"{\"a\":1}\n{\"b\":2}\n{\"c\":").unwrap();
        let mut reader = IncrementalReader::default();
        let mut seen = Vec::new();
        reader.read_appended(&path, 1 << 20, |l| seen.push(String::from_utf8_lossy(l).to_string())).unwrap();
        assert_eq!(seen, vec!["{\"a\":1}", "{\"b\":2}"]);
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"3}\n").unwrap();
        seen.clear();
        reader.read_appended(&path, 1 << 20, |l| seen.push(String::from_utf8_lossy(l).to_string())).unwrap();
        assert_eq!(seen, vec!["{\"c\":3}"]);
        let mut tail = Vec::new();
        read_tail_reverse(&path, 1 << 20, 10, |l| {
            tail.push(String::from_utf8_lossy(l).to_string());
            true
        })
        .unwrap();
        assert_eq!(tail, vec!["{\"c\":3}", "{\"b\":2}", "{\"a\":1}"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
