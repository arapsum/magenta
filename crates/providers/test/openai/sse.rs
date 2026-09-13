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
