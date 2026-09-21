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
