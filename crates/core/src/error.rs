use std::error::Error;

use super::identifiers::ProviderId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderErrorKind {
    AuthenticationRequired,
    PermissionDenied,
    RateLimited,
    InvalidRequest,
    Transport,
    Protocol,
    ServiceUnavailable,
    AgentLimitReached,
    RepeatedToolCalls,
    IncompleteResponse,
    Other,
}

/// Structured, allowlisted context that may be shown or persisted with a
/// provider failure.
///
/// The provider's source error is intentionally not part of this value because
/// it may contain response bodies, prompts, or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderErrorDiagnostic {
    HttpStatus(u16),
    AgentLimits {
        observed_rounds: usize,
        permitted_rounds: usize,
        observed_tool_calls: usize,
        permitted_tool_calls: usize,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("provider generation failed")]
pub struct ProviderError {
    pub provider: ProviderId,
    pub kind: ProviderErrorKind,
    pub diagnostic: Option<ProviderErrorDiagnostic>,
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl ProviderError {
    #[must_use]
    pub fn new(provider: ProviderId, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::Other,
            diagnostic: None,
            source: Box::new(source),
        }
    }

    #[must_use]
    pub fn with_kind(
        provider: ProviderId,
        kind: ProviderErrorKind,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider,
            kind,
            diagnostic: None,
            source: Box::new(source),
        }
    }

    #[must_use]
    pub fn with_kind_and_diagnostic(
        provider: ProviderId,
        kind: ProviderErrorKind,
        diagnostic: ProviderErrorDiagnostic,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider,
            kind,
            diagnostic: Some(diagnostic),
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
        assert_eq!(error.kind, ProviderErrorKind::Other);
        assert_eq!(error.source.to_string(), "connection closed");
    }
}
