use magenta_core::{Message, MessageRole};

pub fn latest_user_prompt(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map(|message| message.content.clone())
}

pub fn fake_response(prompt: &str) -> String {
    let fingerprint = prompt
        .bytes()
        .fold(0usize, |sum, byte| sum.wrapping_add(usize::from(byte)));

    match fingerprint % 3 {
        0 => format!(
            "## A focused direction\n\nI would turn **{prompt}** into a small, observable workflow. Start with the user-visible state, keep the boundary narrow, and let the implementation grow from a real interaction.\n\n- Name the state the user can see.\n- Keep the first action reversible.\n- Measure the result before adding another layer.\n\nThat gives the next decision a clear place to land."
        ),
        1 => format!(
            "Here is a practical shape for **{prompt}**:\n\n1. Capture the intent in one value.\n2. Render the current state immediately.\n3. Move slow work behind a cancellable task.\n4. Persist only after the result is complete.\n\nThe important part is the seam between the UI and the work behind it."
        ),
        _ => format!(
            "I would keep **{prompt}** deliberately small at first. The UI can model the workflow with a typed operation, then a provider or storage adapter can replace the local fixture later.\n\n```rust\nstruct Operation {{\n    input: String,\n    state: OperationState,\n}}\n```\n\nThat shape keeps the interface testable while the remote boundary is still evolving."
        ),
    }
}

pub fn response_chunks(response: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut last_boundary = 0;

    for (index, character) in response.char_indices() {
        let end = index + character.len_utf8();
        if character.is_whitespace() {
            last_boundary = end;
        }
        if end.saturating_sub(start) >= 20 && last_boundary > start {
            let boundary = last_boundary;
            chunks.push(response[start..boundary].to_owned());
            start = boundary;
            last_boundary = boundary;
        }
    }

    if start < response.len() {
        chunks.push(response[start..].to_owned());
    }

    chunks
}

#[cfg(test)]
mod tests {
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
}
