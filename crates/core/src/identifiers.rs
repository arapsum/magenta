#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConversationId(pub u64);

impl ConversationId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

impl MessageId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelId(pub String);

impl ModelId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ProviderId(pub String);

impl ProviderId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_constructed_without_ui_dependencies() {
        assert_eq!(ConversationId::new(7), ConversationId(7));
        assert_eq!(MessageId::new(11), MessageId(11));
        assert_eq!(ModelId::new("sonnet").0, "sonnet");
        assert_eq!(ProviderId::new("anthropic").0, "anthropic");
    }
}
