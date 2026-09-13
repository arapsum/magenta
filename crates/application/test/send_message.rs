use std::path::PathBuf;

use super::*;

#[test]
fn attachment_only_turn_uses_the_first_image_name_for_its_title() {
    let attachments = vec![AttachmentDraft {
        name: "diagram.png".to_owned(),
        source_path: PathBuf::from("diagram.png"),
    }];

    assert_eq!(title_from_prompt("   ", &attachments), "Image: diagram.png");
}

#[test]
fn generated_titles_are_normalized_and_bounded() {
    assert_eq!(
        normalize_generated_title("  **Title:**  Context Budget Design.  "),
        Some("Context Budget Design".to_owned())
    );
    assert_eq!(
        normalize_generated_title("\"Provider-Aware Conversation Titles.\""),
        Some("Provider-Aware Conversation Titles".to_owned())
    );
    assert!(normalize_generated_title("   ").is_none());
    assert!(
        normalize_generated_title(&"x".repeat(80))
            .unwrap()
            .chars()
            .count()
            <= 60
    );
}
