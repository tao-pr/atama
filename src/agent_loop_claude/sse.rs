//! A reader of server-sent events (SSE), the format of the event stream of
//! a session.
//!
//! The stream is text, one field on each line, as in `event: <name>` or
//! `data: <text>`. A blank line ends one message. A line that starts with
//! `:` is a comment, and the server sends comments to keep the connection
//! open. The reader follows the SSE format of the HTML standard, with one
//! exception: at the end of the stream, it also gives the last message if
//! the stream has no blank line after it.

use std::io::{self, BufRead};

/// One message of the stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseMessage {
    /// From the `event:` field. `None` if the message has no such field.
    pub event: Option<String>,
    /// The `data:` lines of the message, joined with `\n`.
    pub data: String,
}

/// Reads [`SseMessage`]s from a stream of bytes.
pub struct SseReader<R> {
    reader: R,
}

impl<R: BufRead> SseReader<R> {
    pub fn new(reader: R) -> Self {
        SseReader { reader }
    }

    /// Reads the next message, or returns `None` at the end of the stream.
    /// It skips comments and messages without data.
    pub fn next_message(&mut self) -> io::Result<Option<SseMessage>> {
        let mut message = SseMessage::default();
        let mut has_data = false;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(has_data.then_some(message));
            }
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if has_data {
                    return Ok(Some(message));
                }
                message = SseMessage::default();
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };
            match field {
                "event" => message.event = Some(value.to_string()),
                "data" => {
                    if has_data {
                        message.data.push('\n');
                    }
                    message.data.push_str(value);
                    has_data = true;
                }
                _ => {}
            }
        }
    }
}
