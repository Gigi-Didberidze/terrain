//! LSP-style message framing (`Content-Length` header + JSON body) over any byte stream.

use std::io::{self, BufRead, Write};

/// Reads one framed message. Returns `Ok(None)` on clean end of stream.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if content_length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "bad Content-Length")
                })?);
            }
        }
    }
    let len = content_length.unwrap_or(0);
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Writes one framed message and flushes.
pub fn write_message(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_messages() {
        let mut buf = Vec::new();
        write_message(&mut buf, br#"{"a":1}"#).unwrap();
        write_message(&mut buf, br#"{"b":2}"#).unwrap();
        let mut r = io::BufReader::new(&buf[..]);
        assert_eq!(read_message(&mut r).unwrap().unwrap(), br#"{"a":1}"#);
        assert_eq!(read_message(&mut r).unwrap().unwrap(), br#"{"b":2}"#);
        assert!(read_message(&mut r).unwrap().is_none());
    }
}
