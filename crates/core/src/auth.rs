use std::{future::Future, pin::Pin};

use super::{ProviderError, ProviderId};

/// A provider-neutral future returned by authentication adapters.
pub type AuthenticationFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'static>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAccount {
    pub provider: ProviderId,
    pub name: Option<String>,
    pub email: Option<String>,
    pub plan: Option<String>,
}

/// A browser authorization that has already reserved its callback listener.
///
/// The UI opens [`Self::authorization_url`] and owns the completion future. If
/// the UI drops the future, the provider must stop waiting for the callback.
pub struct AuthorizationSession {
    pub authorization_url: String,
    pub completion: AuthenticationFuture<ProviderAccount>,
}

pub trait ProviderAuthenticator: Send + Sync {
    fn restore(&self) -> AuthenticationFuture<Option<ProviderAccount>>;

    fn begin_login(&self) -> AuthenticationFuture<AuthorizationSession>;

    fn sign_out(&self) -> AuthenticationFuture<()>;
}
