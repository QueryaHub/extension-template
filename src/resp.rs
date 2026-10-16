//! A very small Redis (RESP2) client. It stands in for "the protocol of your
//! database": replace this module with the client library of the database you
//! are writing the driver for.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

/// One decoded RESP2 reply.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Status(String),
    Int(i64),
    Bulk(Option<String>),
    Array(Vec<Reply>),
}

pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect((host, port))
            .map_err(|e| format!("cannot connect to {host}:{port}: {e}"))?;
        // A driver must never hang the host: always bound the I/O.
        let timeout = Some(Duration::from_secs(10));
        stream
            .set_read_timeout(timeout)
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(timeout)
            .map_err(|e| e.to_string())?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// Sends one command and reads its reply. A server error comes back as `Err`.
    pub fn command(&mut self, args: &[&str]) -> Result<Reply, String> {
        let mut frame = format!("*{}\r\n", args.len());
        for arg in args {
            frame.push_str(&format!("${}\r\n{}\r\n", arg.len(), arg));
        }
        self.writer
            .write_all(frame.as_bytes())
            .map_err(|e| e.to_string())?;
        self.read_reply()
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed by the server".into());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    fn read_reply(&mut self) -> Result<Reply, String> {
        let line = self.read_line()?;
        let (kind, rest) = line.split_at(1);
        match kind {
            "+" => Ok(Reply::Status(rest.to_string())),
            "-" => Err(rest.to_string()),
            ":" => rest
                .parse()
                .map(Reply::Int)
                .map_err(|e| format!("bad integer: {e}")),
            "$" => {
                let len: i64 = rest.parse().map_err(|e| format!("bad length: {e}"))?;
                if len < 0 {
                    return Ok(Reply::Bulk(None));
                }
                let mut buf = vec![0u8; len as usize + 2];
                std::io::Read::read_exact(&mut self.reader, &mut buf).map_err(|e| e.to_string())?;
                buf.truncate(len as usize);
                Ok(Reply::Bulk(Some(
                    String::from_utf8_lossy(&buf).into_owned(),
                )))
            }
            "*" => {
                let len: i64 = rest.parse().map_err(|e| format!("bad length: {e}"))?;
                let mut items = Vec::new();
                for _ in 0..len.max(0) {
                    items.push(self.read_reply()?);
                }
                Ok(Reply::Array(items))
            }
            other => Err(format!("unexpected reply type {other:?}")),
        }
    }
}

/// Splits a command line on spaces; double quotes group words.
pub fn split_command(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.trim().chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_quotes() {
        assert_eq!(
            split_command("GET querya:app:name"),
            ["GET", "querya:app:name"]
        );
        assert_eq!(
            split_command(r#"SET k "two words""#),
            ["SET", "k", "two words"]
        );
        assert!(split_command("   ").is_empty());
    }
}
