//! An incremental parser for server-sent events, the format every model API
//! streams its answers in.
//!
//! Bytes go in as they arrive from the network, in chunks of any size, and
//! complete events come out. The parser follows the event stream format of
//! the HTML standard: lines end with `\n`, `\r\n` or `\r`; `event:` names an
//! event and `data:` lines carry its payload; a blank line ends the event.

/// One event from the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// The event name, empty when the server did not send one.
    pub event: String,
    /// The payload. Several `data:` lines are joined with newlines.
    pub data: String,
}

/// Turns chunks of bytes into [`SseEvent`]s.
#[derive(Debug, Default)]
pub(crate) struct SseParser {
    /// Bytes of a line that has not ended yet.
    line: Vec<u8>,
    event: String,
    data: String,
    has_data: bool,
    /// The last byte was `\r`, so a `\n` right after it belongs to the same
    /// line ending.
    after_carriage_return: bool,
    started: bool,
}

impl SseParser {
    /// A parser at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds more bytes and returns the events they complete.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.after_carriage_return {
                self.after_carriage_return = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\n' => self.end_line(&mut events),
                b'\r' => {
                    self.after_carriage_return = true;
                    self.end_line(&mut events);
                }
                _ => self.line.push(byte),
            }
        }
        events
    }

    /// Ends the stream. Servers are supposed to finish every event with a
    /// blank line; one that did not is returned here rather than lost.
    pub fn finish(&mut self) -> Option<SseEvent> {
        let mut events = Vec::new();
        if !self.line.is_empty() {
            self.end_line(&mut events);
        }
        self.end_line(&mut events);
        events.pop()
    }

    fn end_line(&mut self, events: &mut Vec<SseEvent>) {
        let bytes = std::mem::take(&mut self.line);
        let mut line = String::from_utf8_lossy(&bytes);
        if !self.started {
            self.started = true;
            if let Some(without_mark) = line.strip_prefix('\u{feff}') {
                line = without_mark.to_owned().into();
            }
        }

        if line.is_empty() {
            // A blank line dispatches the event, if there is one.
            if self.has_data {
                let mut data = std::mem::take(&mut self.data);
                data.pop(); // the newline after the last data line
                events.push(SseEvent {
                    event: std::mem::take(&mut self.event),
                    data,
                });
            }
            self.event.clear();
            self.data.clear();
            self.has_data = false;
            return;
        }
        if line.starts_with(':') {
            return; // a comment, used by servers as a keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => self.event = value.to_owned(),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
                self.has_data = true;
            }
            // `id` and `retry` are for reconnecting, which chat requests
            // never do.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, data: &str) -> SseEvent {
        SseEvent {
            event: name.to_owned(),
            data: data.to_owned(),
        }
    }

    #[test]
    fn parses_named_events() {
        let mut parser = SseParser::new();
        let events =
            parser.push(b"event: message_start\ndata: {\"a\":1}\n\nevent: ping\ndata: {}\n\n");
        assert_eq!(
            events,
            [event("message_start", "{\"a\":1}"), event("ping", "{}")]
        );
    }

    #[test]
    fn events_survive_being_split_at_every_byte() {
        let stream = "event: delta\ndata: {\"text\":\"h\u{e9}llo \u{2713}\"}\n\ndata: [DONE]\n\n";
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        for byte in stream.as_bytes() {
            events.extend(parser.push(&[*byte]));
        }
        assert_eq!(
            events,
            [
                event("delta", "{\"text\":\"h\u{e9}llo \u{2713}\"}"),
                event("", "[DONE]")
            ]
        );
    }

    #[test]
    fn all_three_line_endings_are_understood() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: one\r\n\r\ndata: two\r\rdata: three\n\n");
        assert_eq!(
            events,
            [event("", "one"), event("", "two"), event("", "three")]
        );
    }

    #[test]
    fn a_line_ending_split_between_chunks_is_one_ending() {
        let mut parser = SseParser::new();
        let mut events = parser.push(b"data: one\r");
        events.extend(parser.push(b"\n\r"));
        events.extend(parser.push(b"\ndata: two\n\n"));
        assert_eq!(events, [event("", "one"), event("", "two")]);
    }

    #[test]
    fn several_data_lines_are_joined_with_newlines() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: first\ndata: second\ndata:\n\n");
        assert_eq!(events, [event("", "first\nsecond\n")]);
    }

    #[test]
    fn comments_and_unknown_fields_are_ignored() {
        let mut parser = SseParser::new();
        let events = parser.push(b": keep-alive\nid: 7\nretry: 1000\ndata: x\n\n: again\n\n");
        assert_eq!(events, [event("", "x")]);
    }

    #[test]
    fn a_value_keeps_everything_after_the_first_space() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data:  two spaces\ndata:no space\ndata: a: b\n\n");
        assert_eq!(events, [event("", " two spaces\nno space\na: b")]);
    }

    #[test]
    fn an_event_name_without_data_is_not_an_event() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"event: lonely\n\n").is_empty());
        // And the name does not leak into the next event.
        assert_eq!(parser.push(b"data: x\n\n"), [event("", "x")]);
    }

    #[test]
    fn a_byte_order_mark_at_the_start_is_dropped() {
        let mut parser = SseParser::new();
        let events = parser.push("\u{feff}data: x\n\n".as_bytes());
        assert_eq!(events, [event("", "x")]);
    }

    #[test]
    fn finish_returns_an_event_the_server_did_not_terminate() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"event: last\ndata: tail").is_empty());
        assert_eq!(parser.finish(), Some(event("last", "tail")));
        assert_eq!(parser.finish(), None);
    }

    #[test]
    fn finish_after_a_complete_stream_returns_nothing() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push(b"data: x\n\n").len(), 1);
        assert_eq!(parser.finish(), None);
    }
}
