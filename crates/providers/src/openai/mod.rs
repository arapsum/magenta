mod agent;
mod auth;
mod chat;
mod commands;
mod http;
mod sse;
mod wire;

use chat::{
    assistant_text_phase, classify_status, provider_error, response_error_detail,
    response_error_message,
};

use std::{sync::Arc, time::Duration};

use futures_util::{
    StreamExt as _,
    io::{AsyncBufReadExt as _, BufReader},
};
use http_client::{HttpClient, Method, StatusCode};
use magenta_core::{
    AssistantTextPhase, AuthenticationFuture, AuthorizationSession, ChatProvider, GenerationEvent,
    GenerationOutcome, GenerationRequest, GenerationStream, ModelCatalog, ModelCatalogFuture,
    ModelDescriptor, ProviderAccount, ProviderAuthenticator, ProviderError,
    ProviderErrorDiagnostic, ProviderErrorKind,
};
use reqwest_client::ReqwestClient;
use serde_json::Value;
use url::Url;

use self::{
    auth::{OpenAiAuth, openai_provider},
    sse::EventDecoder,
    wire::{
        ModelsResponse, ResponseError, ResponsesRequest, StreamEvent, model_descriptors,
        parse_finish_reason, reasoning_summary_parts, usage,
    },
};

const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
// The Codex catalog filters models by its own client compatibility version;
// Magenta's package version is intentionally independent from that contract.
// Keep this in sync with the known-good Codex wire version used by oh-my-pi.
const CLIENT_VERSION: &str = "0.153.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_TIMEOUT: Duration = Duration::from_mins(30);
const MAX_ERROR_BODY: usize = 128 * 1024;
const MAX_MODEL_BODY: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
enum OpenAiProviderError {
    #[error("could not serialize the OpenAI request: {0}")]
    Serialize(String),
    #[error("could not build the OpenAI request: {0}")]
    RequestBuild(String),
    #[error("OpenAI request transport failed: {0}")]
    Transport(String),
    #[error("OpenAI returned HTTP {status}: {detail}")]
    Http { status: u16, detail: String },
    #[error("OpenAI returned malformed streaming data: {0}")]
    Protocol(String),
    #[error("OpenAI ended the stream before a completion event")]
    IncompleteStream,
    #[error("OpenAI rejected the stream: {0}")]
    StreamFailed(String),
}

#[derive(Clone)]
pub struct OpenAiProvider {
    client: Arc<dyn HttpClient>,
    auth: Arc<OpenAiAuth>,
    base_url: String,
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiProvider {
    #[must_use]
    pub fn new() -> Self {
        let client: Arc<dyn HttpClient> = Arc::new(ReqwestClient::new());
        Self::with_client(client)
    }

    fn with_client(client: Arc<dyn HttpClient>) -> Self {
        let auth = Arc::new(OpenAiAuth::new(Arc::clone(&client)));
        Self {
            client,
            auth,
            base_url: CODEX_BASE_URL.to_owned(),
        }
    }

    async fn models_inner(&self) -> Result<Vec<ModelDescriptor>, ProviderError> {
        let mut access_token = self.auth.access_token().await?;
        let mut response = self.send_models(&access_token).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            access_token = self.auth.force_refresh(&access_token).await?;
            response = self.send_models(&access_token).await?;
        }

        if !response.status().is_success() {
            return Err(self.http_error(response).await);
        }

        let mut body = response.into_body();
        let bytes = http::read_limited(&mut body, MAX_MODEL_BODY)
            .await
            .map_err(|error| {
                provider_error(
                    ProviderErrorKind::Transport,
                    OpenAiProviderError::Transport(error.to_string()),
                )
            })?;
        let response = serde_json::from_slice::<ModelsResponse>(&bytes).map_err(|error| {
            provider_error(
                ProviderErrorKind::Protocol,
                OpenAiProviderError::Protocol(error.to_string()),
            )
        })?;
        let models = model_descriptors(response);
        if models.is_empty() {
            return Err(provider_error(
                ProviderErrorKind::Protocol,
                OpenAiProviderError::Protocol("the model catalog was empty".to_owned()),
            ));
        }
        Ok(models)
    }

    async fn send_models(
        &self,
        access_token: &str,
    ) -> Result<http_client::Response<http_client::AsyncBody>, ProviderError> {
        let mut url = Url::parse(&self.endpoint("models")).map_err(|error| {
            provider_error(
                ProviderErrorKind::InvalidRequest,
                OpenAiProviderError::RequestBuild(error.to_string()),
            )
        })?;
        url.query_pairs_mut()
            .append_pair("client_version", CLIENT_VERSION);
        let account_id = self.auth.account_id().await;
        let authorization = format!("Bearer {access_token}");
        let headers = Self::headers(
            &authorization,
            account_id.as_deref(),
            "application/json",
            None,
        );
        let request = http::request(
            Method::GET,
            url.as_str(),
            &headers,
            Vec::new(),
            REQUEST_TIMEOUT,
        )
        .map_err(|error| {
            provider_error(
                ProviderErrorKind::InvalidRequest,
                OpenAiProviderError::RequestBuild(error.to_string()),
            )
        })?;
        http::send(self.client.as_ref(), request)
            .await
            .map_err(|error| {
                provider_error(
                    ProviderErrorKind::Transport,
                    OpenAiProviderError::Transport(error),
                )
            })
    }

    async fn send_responses(
        &self,
        access_token: &str,
        request: &ResponsesRequest,
    ) -> Result<http_client::Response<http_client::AsyncBody>, ProviderError> {
        let body = serde_json::to_vec(request).map_err(|error| {
            provider_error(
                ProviderErrorKind::InvalidRequest,
                OpenAiProviderError::Serialize(error.to_string()),
            )
        })?;
        let authorization = format!("Bearer {access_token}");
        let account_id = self.auth.account_id().await;
        let routing_hint = format!("model={}", request.model);
        let headers = Self::headers(
            &authorization,
            account_id.as_deref(),
            "text/event-stream",
            Some(&routing_hint),
        );
        let request = http::request(
            Method::POST,
            &self.endpoint("responses"),
            &headers,
            body,
            STREAM_TIMEOUT,
        )
        .map_err(|error| {
            provider_error(
                ProviderErrorKind::InvalidRequest,
                OpenAiProviderError::RequestBuild(error.to_string()),
            )
        })?;
        http::send(self.client.as_ref(), request)
            .await
            .map_err(|error| {
                provider_error(
                    ProviderErrorKind::Transport,
                    OpenAiProviderError::Transport(error),
                )
            })
    }

    fn headers<'a>(
        authorization: &'a str,
        account_id: Option<&'a str>,
        accept: &'a str,
        routing_hint: Option<&'a str>,
    ) -> Vec<(&'static str, &'a str)> {
        let mut headers = vec![
            ("accept", accept),
            ("authorization", authorization),
            ("content-type", "application/json"),
            ("openai-beta", "responses=experimental"),
            ("originator", "omp"),
            ("version", CLIENT_VERSION),
        ];
        if let Some(account_id) = account_id {
            headers.push(("chatgpt-account-id", account_id));
        }
        if let Some(routing_hint) = routing_hint {
            headers.push(("x-codex-routing-hint", routing_hint));
        }
        headers
    }

    fn endpoint(&self, resource: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            resource.trim_start_matches('/')
        )
    }

    async fn http_error(
        &self,
        mut response: http_client::Response<http_client::AsyncBody>,
    ) -> ProviderError {
        let status = response.status().as_u16();
        let detail = match http::read_limited(response.body_mut(), MAX_ERROR_BODY).await {
            Ok(body) => response_error_message(&body),
            Err(error) => format!("could not read the error response: {error}"),
        };
        provider_error(
            classify_status(status),
            OpenAiProviderError::Http { status, detail },
        )
    }
}
