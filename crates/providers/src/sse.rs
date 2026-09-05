#[derive(Debug, Default)]
pub struct EventDecoder {
    event: Option<String>,
    data: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ServerSentEvent {
    pub event: String,
    pub data: String,
}

impl EventDecoder {
    pub fn push_line(&mut self, line: &str) -> Option<ServerSentEvent> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);

        if line.is_empty() {
            return self.finish();
        }

        if line.starts_with(':') {
            return None;
        }

        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            (field, value.strip_prefix(' ').unwrap_or(value))
        });

        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }

        None
    }

    pub fn finish(&mut self) -> Option<ServerSentEvent> {
        if self.data.is_empty() {
            self.event = None;
            return None;
        }

        Some(ServerSentEvent {
            event: self.event.take().unwrap_or_else(|| "message".to_owned()),
            data: std::mem::take(&mut self.data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_joins_data_lines_and_resets_on_blank_line() {
        let mut decoder = EventDecoder::default();

        assert_eq!(
            decoder.push_line("event: response.output_text.delta\n"),
            None
        );
        assert_eq!(decoder.push_line("data: {\n"), None);
        assert_eq!(decoder.push_line("data: \"delta\":\"hello\"}\n"), None);
        assert_eq!(
            decoder.push_line("\n"),
            Some(ServerSentEvent {
                event: "response.output_text.delta".to_owned(),
                data: "{\n\"delta\":\"hello\"}".to_owned(),
            })
        );
        assert_eq!(decoder.push_line(": keep-alive\n"), None);
    }

    #[test]
    fn decoder_supports_data_without_a_space_after_the_colon() {
        let mut decoder = EventDecoder::default();

        assert_eq!(decoder.push_line("data:{}\r\n"), None);
        assert_eq!(
            decoder.push_line("\r\n"),
            Some(ServerSentEvent {
                event: "message".to_owned(),
                data: "{}".to_owned(),
            })
        );
    }
}
