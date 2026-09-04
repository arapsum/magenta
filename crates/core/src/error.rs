use std::error::Error;

use super::identifiers::ProviderId;

#[derive(Debug, thiserror::Error)]
#[error("provider generation failed")]
pub struct ProviderError {
    pub provider: ProviderId,
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl ProviderError {
    #[must_use]
    pub fn new(provider: ProviderId, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            provider,
            source: Box::new(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_preserve_the_provider_and_source() {
        let source = std::io::Error::other("connection closed");
        let error = ProviderError::new(ProviderId::new("anthropic"), source);

        assert_eq!(error.provider, ProviderId::new("anthropic"));
        assert_eq!(error.source.to_string(), "connection closed");
    }
}
