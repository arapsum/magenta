mod agent;
mod auth;
mod http;
mod sse;
mod wire;

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

    fn stream_event(event: &StreamEvent) -> Result<Option<StreamOutput>, OpenAiProviderError> {
        match event.kind.as_str() {
            "response.output_text.delta" | "response.refusal.delta" => Ok(event
                .delta
                .as_deref()
                .filter(|delta| !delta.is_empty())
                .map(|delta| {
                    event
                        .phase
                        .as_deref()
                        .and_then(assistant_text_phase)
                        .map_or_else(
                            || StreamOutput::Text(delta.to_owned()),
                            |phase| StreamOutput::TextWithPhase {
                                delta: delta.to_owned(),
                                phase,
                            },
                        )
                })),
            "response.reasoning_summary_part.added" => {
                let valid_part = event.part.as_ref().is_none_or(|part| {
                    part.get("type").and_then(Value::as_str) == Some("summary_text")
                });
                if !valid_part {
                    return Ok(None);
                }
                summary_key(event).map_or_else(
                    || Ok(None),
                    |key| {
                        Ok(Some(StreamOutput::ReasoningSummaryStarted {
                            key,
                            title: "Thinking".to_owned(),
                        }))
                    },
                )
            }
            "response.reasoning_summary_text.delta" => summary_key(event).map_or_else(
                || Ok(None),
                |key| {
                    Ok(event
                        .delta
                        .as_deref()
                        .filter(|delta| !delta.is_empty())
                        .map(|delta| StreamOutput::ReasoningSummaryDelta {
                            key,
                            delta: delta.to_owned(),
                        }))
                },
            ),
            "response.reasoning_summary_text.done" => summary_key(event).map_or_else(
                || Ok(None),
                |key| {
                    Ok(Some(StreamOutput::ReasoningSummaryCompleted {
                        key,
                        text: event.text.clone().unwrap_or_default(),
                    }))
                },
            ),
            "response.completed" => {
                let response = event.response.as_ref().ok_or_else(|| {
                    OpenAiProviderError::Protocol(
                        "response.completed did not contain a response payload".to_owned(),
                    )
                })?;
                Ok(Some(StreamOutput::Completed(GenerationOutcome::new(
                    parse_finish_reason(response),
                    usage(response),
                ))))
            }
            "response.incomplete" => {
                let response = event.response.as_ref().ok_or_else(|| {
                    OpenAiProviderError::Protocol(
                        "response.incomplete did not contain a response payload".to_owned(),
                    )
                })?;
                Ok(Some(StreamOutput::Completed(GenerationOutcome::new(
                    parse_finish_reason(response),
                    usage(response),
                ))))
            }
            "response.failed" | "error" | "response.error" => {
                let error = event
                    .error
                    .as_ref()
                    .or_else(|| {
                        event
                            .response
                            .as_ref()
                            .and_then(|response| response.error.as_ref())
                    })
                    .map_or_else(
                        || "the provider reported an unspecified error".to_owned(),
                        response_error_detail,
                    );
                Err(OpenAiProviderError::StreamFailed(error))
            }
            _ => Ok(None),
        }
    }

    async fn stream_inner(
        &self,
        request: GenerationRequest,
    ) -> Result<
        impl futures_util::Stream<Item = Result<GenerationEvent, ProviderError>>,
        ProviderError,
    > {
        let wire_request = smol::unblock(move || {
            ResponsesRequest::from_request(
                request.generation.model.0.as_str(),
                &request.generation.effort,
                &request.messages,
            )
        })
        .await
        .map_err(|message| {
            provider_error(
                ProviderErrorKind::InvalidRequest,
                OpenAiProviderError::Protocol(message),
            )
        })?;
        let mut access_token = self.auth.access_token().await?;
        let mut response = self.send_responses(&access_token, &wire_request).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            access_token = self.auth.force_refresh(&access_token).await?;
            response = self.send_responses(&access_token, &wire_request).await?;
        }
        if !response.status().is_success() {
            return Err(self.http_error(response).await);
        }

        Ok(generation_response_stream(response))
    }
}

fn generation_response_stream(
    response: http_client::Response<http_client::AsyncBody>,
) -> impl futures_util::Stream<Item = Result<GenerationEvent, ProviderError>> {
    async_stream::try_stream! {
        let mut decoder = EventDecoder::default();
        let mut reader = BufReader::new(response.into_body());
        let mut line = String::new();
        let mut completed = false;
        let mut state = ChatStreamState::default();
        yield GenerationEvent::Started;

        loop {
            line.clear();
            let count = reader.read_line(&mut line).await.map_err(|error| {
                provider_error(
                    ProviderErrorKind::Transport,
                    OpenAiProviderError::Transport(error.to_string()),
                )
            })?;
            if count == 0 {
                break;
            }
            if let Some(event) = decoder.push_line(&line) {
                ensure_stream_is_active(completed)?;
                let event = parse_stream_event(&event.data)?;
                for output in state.observe(&event)? {
                    let is_completed = matches!(&output, StreamOutput::Completed(_));
                    yield generation_event(output);
                    if is_completed {
                        completed = true;
                    }
                }
            }
        }

        if let Some(event) = decoder.finish() {
            ensure_stream_is_active(completed)?;
            let event = parse_stream_event(&event.data)?;
            for output in state.observe(&event)? {
                let is_completed = matches!(&output, StreamOutput::Completed(_));
                yield generation_event(output);
                if is_completed {
                    completed = true;
                }
            }
        }

        if !completed {
            Err::<(), _>(provider_error(
                ProviderErrorKind::IncompleteResponse,
                OpenAiProviderError::IncompleteStream,
            ))?;
        }
    }
}

fn parse_stream_event(data: &str) -> Result<StreamEvent, ProviderError> {
    serde_json::from_str::<StreamEvent>(data).map_err(|error| {
        provider_error(
            ProviderErrorKind::Protocol,
            OpenAiProviderError::Protocol(error.to_string()),
        )
    })
}

fn generation_event(output: StreamOutput) -> GenerationEvent {
    match output {
        StreamOutput::Text(delta) => GenerationEvent::TextDelta(delta),
        StreamOutput::TextWithPhase { delta, phase } => {
            GenerationEvent::TextDeltaWithPhase { delta, phase }
        }
        StreamOutput::ReasoningSummaryStarted { key, title } => {
            GenerationEvent::ReasoningSummaryStarted { key, title }
        }
        StreamOutput::ReasoningSummaryDelta { key, delta } => {
            GenerationEvent::ReasoningSummaryDelta { key, delta }
        }
        StreamOutput::ReasoningSummaryCompleted { key, text } => {
            GenerationEvent::ReasoningSummaryCompleted { key, text }
        }
        StreamOutput::Completed(outcome) => GenerationEvent::Completed(outcome),
    }
}

impl ChatProvider for OpenAiProvider {
    fn stream(&self, request: GenerationRequest) -> GenerationStream {
        let provider = self.clone();
        Box::pin(async_stream::try_stream! {
            let stream = provider.stream_inner(request).await?;
            futures_util::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                yield event?;
            }
        })
    }
}

impl ModelCatalog for OpenAiProvider {
    fn models(&self) -> ModelCatalogFuture {
        let provider = self.clone();
        Box::pin(async move { provider.models_inner().await })
    }
}

impl ProviderAuthenticator for OpenAiProvider {
    fn restore(&self) -> AuthenticationFuture<Option<ProviderAccount>> {
        self.auth.restore()
    }

    fn begin_login(&self) -> AuthenticationFuture<AuthorizationSession> {
        self.auth.begin_login()
    }

    fn sign_out(&self) -> AuthenticationFuture<()> {
        self.auth.sign_out()
    }
}

#[derive(Clone)]
enum StreamOutput {
    Text(String),
    TextWithPhase {
        delta: String,
        phase: AssistantTextPhase,
    },
    ReasoningSummaryStarted {
        key: String,
        title: String,
    },
    ReasoningSummaryDelta {
        key: String,
        delta: String,
    },
    ReasoningSummaryCompleted {
        key: String,
        text: String,
    },
    Completed(GenerationOutcome),
}

#[derive(Default)]
struct ChatStreamState {
    phases: std::collections::HashMap<String, AssistantTextPhase>,
    summaries: std::collections::HashMap<String, ChatSummaryState>,
}

#[derive(Default)]
struct ChatSummaryState {
    text: String,
    started: bool,
    completed: bool,
}

impl ChatStreamState {
    fn observe(&mut self, event: &StreamEvent) -> Result<Vec<StreamOutput>, ProviderError> {
        if matches!(
            event.kind.as_str(),
            "response.output_item.added" | "response.output_item.done"
        ) && let Some(item) = event.item.as_ref()
            && item.get("type").and_then(Value::as_str) == Some("message")
            && let Some(id) = item.get("id").and_then(Value::as_str)
            && let Some(phase) = item
                .get("phase")
                .and_then(Value::as_str)
                .and_then(assistant_text_phase)
        {
            self.phases.insert(id.to_owned(), phase);
        }

        if event.kind == "response.output_text.delta"
            && event.phase.is_none()
            && let Some(item_id) = event.item_id.as_deref()
            && let Some(phase) = self.phases.get(item_id).copied()
        {
            let mut outputs = Vec::new();
            if let Some(delta) = event.delta.as_deref().filter(|delta| !delta.is_empty()) {
                outputs.push(StreamOutput::TextWithPhase {
                    delta: delta.to_owned(),
                    phase,
                });
            }
            return Ok(outputs);
        }

        let decoded = OpenAiProvider::stream_event(event)
            .map_err(|error| provider_error(ProviderErrorKind::Protocol, error))?;
        let terminal = decoded
            .clone()
            .filter(|output| matches!(output, StreamOutput::Completed(_)));
        let mut outputs = Vec::new();
        if let Some(output) = decoded.filter(|output| !matches!(output, StreamOutput::Completed(_)))
        {
            if let Some(key) = match &output {
                StreamOutput::ReasoningSummaryDelta { key, .. }
                | StreamOutput::ReasoningSummaryCompleted { key, .. } => Some(key),
                _ => None,
            }
            .filter(|key| !self.summaries.get(*key).is_some_and(|state| state.started))
            {
                outputs.push(StreamOutput::ReasoningSummaryStarted {
                    key: key.clone(),
                    title: "Thinking".to_owned(),
                });
            }
            self.observe_summary_output(&output);
            outputs.push(output);
        }

        if matches!(
            event.kind.as_str(),
            "response.completed" | "response.incomplete"
        ) && let Some(response) = event.response.as_ref()
        {
            for (item_id, summary_index, text) in reasoning_summary_parts(&response.output) {
                let key = format!("reasoning:{item_id}:{summary_index}");
                let state = self.summaries.entry(key.clone()).or_default();
                if state.completed {
                    continue;
                }
                if !state.started {
                    outputs.push(StreamOutput::ReasoningSummaryStarted {
                        key: key.clone(),
                        title: "Thinking".to_owned(),
                    });
                    state.started = true;
                }
                state.text.clone_from(&text);
                state.completed = true;
                outputs.push(StreamOutput::ReasoningSummaryCompleted { key, text });
            }
        }

        if let Some(terminal) = terminal {
            outputs.push(terminal);
        }

        Ok(outputs)
    }

    fn observe_summary_output(&mut self, output: &StreamOutput) {
        match output {
            StreamOutput::ReasoningSummaryStarted { key, .. } => {
                self.summaries.entry(key.clone()).or_default().started = true;
            }
            StreamOutput::ReasoningSummaryDelta { key, delta } => {
                let state = self.summaries.entry(key.clone()).or_default();
                state.started = true;
                state.text.push_str(delta);
            }
            StreamOutput::ReasoningSummaryCompleted { key, text } => {
                let state = self.summaries.entry(key.clone()).or_default();
                state.started = true;
                if !text.is_empty() {
                    state.text.clone_from(text);
                }
                state.completed = true;
            }
            _ => {}
        }
    }
}

fn summary_key(event: &StreamEvent) -> Option<String> {
    Some(format!(
        "reasoning:{}:{}",
        event.item_id.as_deref()?,
        event.summary_index?
    ))
}

fn assistant_text_phase(value: &str) -> Option<AssistantTextPhase> {
    match value {
        "commentary" => Some(AssistantTextPhase::Commentary),
        "final_answer" => Some(AssistantTextPhase::FinalAnswer),
        _ => None,
    }
}

fn ensure_stream_is_active(completed: bool) -> Result<(), ProviderError> {
    if completed {
        return Err(provider_error(
            ProviderErrorKind::Protocol,
            OpenAiProviderError::Protocol("received data after completion".to_owned()),
        ));
    }
    Ok(())
}

fn response_error_detail(error: &ResponseError) -> String {
    match (&error.code, &error.message) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (None, Some(message)) => message.clone(),
        (Some(code), None) => code.clone(),
        (None, None) => "the provider reported an unspecified error".to_owned(),
    }
}

fn response_error_message(body: &[u8]) -> String {
    let value = serde_json::from_slice::<Value>(body).ok();
    value
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .or_else(|| {
            value
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)
        .or_else(|| String::from_utf8(body.to_vec()).ok())
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "the provider returned an empty error response".to_owned())
}

const fn classify_status(status: u16) -> ProviderErrorKind {
    match status {
        401 => ProviderErrorKind::AuthenticationRequired,
        403 => ProviderErrorKind::PermissionDenied,
        408 | 429 => ProviderErrorKind::RateLimited,
        400 | 404 | 422 => ProviderErrorKind::InvalidRequest,
        500..=599 => ProviderErrorKind::ServiceUnavailable,
        _ => ProviderErrorKind::Other,
    }
}

fn provider_error(kind: ProviderErrorKind, error: OpenAiProviderError) -> ProviderError {
    let diagnostic = match &error {
        OpenAiProviderError::Http { status, .. } => {
            Some(ProviderErrorDiagnostic::HttpStatus(*status))
        }
        _ => None,
    };
    if let Some(diagnostic) = diagnostic {
        ProviderError::with_kind_and_diagnostic(openai_provider(), kind, diagnostic, error)
    } else {
        ProviderError::with_kind(openai_provider(), kind, error)
    }
}

#[cfg(test)]
#[path = "../../test/openai/mod.rs"]
mod tests;
