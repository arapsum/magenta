use super::*;

#[test]
fn identifiers_are_constructed_without_ui_dependencies() {
    assert_eq!(ConversationId::new(7), ConversationId(7));
    assert_eq!(MessageId::new(11), MessageId(11));
    assert_eq!(ModelId::new("sonnet").0, "sonnet");
    assert_eq!(ProviderId::new("anthropic").0, "anthropic");
}
