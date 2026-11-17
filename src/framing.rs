//! Bounded newline framing for the JSON-line transport (#33, #7).
//!
//! Requests are split on `\n` at the BYTE level, with at most `max_bytes` buffered per frame,
//! before any UTF-8 decoding. That keeps three outcomes distinct that `BufRead::lines`
//! conflated: a well-formed frame, an isolated malformed frame (oversized or invalid UTF-8,
//! both recoverable at the next delimiter), and the end of input (clean EOF vs a genuine read
//! error). Protocol semantics stay in Core; this only turns bytes into frames.

use std::io::{self, BufRead};

#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// One complete UTF-8 frame, without its `\n` / `\r\n` terminator.
    Line(String),
    /// The frame exceeded the byte limit; its remainder was discarded up to the next `\n`.
    TooLarge,
    /// The frame was not valid UTF-8; it was discarded in full.
    InvalidUtf8,
    /// Clean end of input.
    Eof,
}

/// Read the next frame, buffering at most `max_bytes` of it. `Err` is a genuine I/O failure
/// of the underlying reader (`Interrupted` is retried).
pub fn read_frame(reader: &mut impl BufRead, max_bytes: usize) -> io::Result<Frame> {
    let mut buf: Vec<u8> = Vec::new();
    let mut oversized = false;
    let mut saw_any = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            // EOF: a final unterminated frame is still a frame.
            if !saw_any {
                return Ok(Frame::Eof);
            }
            break;
        }
        saw_any = true;
        let (chunk, consumed, done) = match available.iter().position(|&b| b == b'\n') {
            Some(index) => (&available[..index], index + 1, true),
            None => (available, available.len(), false),
        };
        if !oversized {
            // One spare byte so a trailing `\r` does not count against the limit.
            if buf.len() + chunk.len() > max_bytes.saturating_add(1) {
                oversized = true;
                buf = Vec::new(); // release what was buffered; keep discarding to the delimiter
            } else {
                buf.extend_from_slice(chunk);
            }
        }
        reader.consume(consumed);
        if done {
            break;
        }
    }
    if oversized {
        return Ok(Frame::TooLarge);
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    if buf.len() > max_bytes {
        return Ok(Frame::TooLarge);
    }
    Ok(match String::from_utf8(buf) {
        Ok(text) => Frame::Line(text),
        Err(_) => Frame::InvalidUtf8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor, Read};

    fn frames(input: &[u8], max: usize) -> Vec<Frame> {
        // A tiny buffer capacity exercises frames spanning many fill_buf calls.
        let mut reader = BufReader::with_capacity(3, Cursor::new(input.to_vec()));
        let mut out = Vec::new();
        loop {
            let frame = read_frame(&mut reader, max).unwrap();
            let end = frame == Frame::Eof;
            out.push(frame);
            if end {
                return out;
            }
        }
    }

    fn line(s: &str) -> Frame {
        Frame::Line(s.to_owned())
    }

    #[test]
    fn splits_lines_and_strips_terminators() {
        assert_eq!(frames(b"a\nbc\r\n\nd", 16), vec![line("a"), line("bc"), line(""), line("d"), Frame::Eof]);
        assert_eq!(frames(b"", 16), vec![Frame::Eof]);
    }

    #[test]
    fn invalid_utf8_is_one_isolated_frame() {
        assert_eq!(frames(b"\xff\nok\n", 16), vec![Frame::InvalidUtf8, line("ok"), Frame::Eof]);
        assert_eq!(frames(b"\"a\xffb\"\nok", 16), vec![Frame::InvalidUtf8, line("ok"), Frame::Eof]);
        // A multi-byte character split across buffer refills is still valid.
        assert_eq!(frames("é€\n".as_bytes(), 16), vec![line("é€"), Frame::Eof]);
    }

    #[test]
    fn oversized_frame_is_bounded_and_recovers_at_the_delimiter() {
        assert_eq!(frames(b"12345\n1234\nx", 4), vec![Frame::TooLarge, line("1234"), line("x"), Frame::Eof]);
        assert_eq!(frames(b"123456789", 4), vec![Frame::TooLarge, Frame::Eof]);
        assert_eq!(frames(b"12345", 4), vec![Frame::TooLarge, Frame::Eof]);
        // The limit applies to the frame, not its terminator.
        assert_eq!(frames(b"1234\r\n", 4), vec![line("1234"), Frame::Eof]);
    }

    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "boom"))
        }
    }

    #[test]
    fn read_errors_are_not_reported_as_eof() {
        let error = read_frame(&mut BufReader::new(Failing), 16).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    struct InterruptOnce(bool, Cursor<Vec<u8>>);
    impl Read for InterruptOnce {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "signal"));
            }
            self.1.read(buf)
        }
    }

    #[test]
    fn interrupted_reads_are_retried() {
        let mut reader = BufReader::new(InterruptOnce(false, Cursor::new(b"x\n".to_vec())));
        assert_eq!(read_frame(&mut reader, 16).unwrap(), line("x"));
    }
}
