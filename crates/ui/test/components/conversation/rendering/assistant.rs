use chrono::Duration;

use super::{display_model_name, relative_message_timestamp};

#[test]
fn gpt_model_names_use_the_product_casing() {
    assert_eq!(display_model_name("gpt-5.6-luna"), "GPT-5.6-luna");
    assert_eq!(display_model_name("GpT-5.5"), "GPT-5.5");
    assert_eq!(display_model_name("claude-sonnet"), "claude-sonnet");
}

#[test]
fn recent_message_timestamps_are_human_readable() {
    let timestamp =
        magenta_core::Timestamp((chrono::Local::now() - Duration::minutes(2)).timestamp_millis());

    assert_eq!(relative_message_timestamp(timestamp), "2m ago");
}
