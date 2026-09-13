use super::*;

#[test]
fn response_chunks_preserve_markdown_and_unicode() {
    let response = "A calm café with **bold** detail.";
    let chunks = response_chunks(response);

    assert!(!chunks.is_empty());
    assert_eq!(chunks.concat(), response);
}

#[test]
fn fake_responses_are_deterministic_and_rich() {
    let response = fake_response("streaming responses in GPUI");

    assert_eq!(response, fake_response("streaming responses in GPUI"));
    assert!(response.contains("**streaming responses in GPUI**"));
}
