use super::*;

#[test]
fn provider_errors_preserve_the_provider_and_source() {
    let source = std::io::Error::other("connection closed");
    let error = ProviderError::new(ProviderId::new("anthropic"), source);

    assert_eq!(error.provider, ProviderId::new("anthropic"));
    assert_eq!(error.kind, ProviderErrorKind::Other);
    assert_eq!(error.source.to_string(), "connection closed");
}
