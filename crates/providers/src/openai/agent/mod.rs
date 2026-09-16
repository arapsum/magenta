mod stream;

use std::collections::HashMap;

use futures_util::{
    StreamExt as _,
    io::{AsyncBufReadExt as _, BufReader},
};
use http_client::StatusCode;
use magenta_core::{
    AgentContinuation, AgentProvider, AgentProviderEvent, AgentProviderStream, AgentRequest,
    AgentResumeRequest, AgentToolCall, AssistantTextPhase, GenerationOutcome, ProviderError,
    ProviderErrorKind,
};

use super::{
    OpenAiProvider, StreamEvent, assistant_text_phase,
    wire::{ResponsePayload, ResponsesRequest, reasoning_summary_parts},
};

impl AgentProvider for OpenAiProvider {
    fn start(&self, request: AgentRequest) -> AgentProviderStream {
        let provider = self.clone();
        Box::pin(async_stream::try_stream! {
            let wire_request = ResponsesRequest::from_agent_request(&request).map_err(|message| {
                super::provider_error(
                    ProviderErrorKind::InvalidRequest,
                    super::OpenAiProviderError::Protocol(message),
                )
            })?;
            let stream = provider.agent_stream_inner(
                wire_request,
                request.generation.model.0,
                request.generation.effort,
            ).await?;
            futures_util::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                yield event?;
            }
        })
    }

    fn resume(&self, request: AgentResumeRequest) -> AgentProviderStream {
        let provider = self.clone();
        Box::pin(async_stream::try_stream! {
            let wire_request = ResponsesRequest::from_resume(
                &request,
                &request.continuation.model.0,
                &request.continuation.effort,
            ).map_err(|message| {
                super::provider_error(
                    ProviderErrorKind::InvalidRequest,
                    super::OpenAiProviderError::Protocol(message),
                )
            })?;
            let stream = provider.agent_stream_inner(
                wire_request,
                request.continuation.model.0,
                request.continuation.effort,
            ).await?;
            futures_util::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                yield event?;
            }
        })
    }
}

impl OpenAiProvider {
    async fn agent_stream_inner(
        &self,
        request: ResponsesRequest,
        model: String,
        effort: magenta_core::EffortLevel,
    ) -> Result<
        impl futures_util::Stream<Item = Result<AgentProviderEvent, ProviderError>>,
        ProviderError,
    > {
        let input = request.input.as_array().cloned().ok_or_else(|| {
            super::provider_error(
                ProviderErrorKind::InvalidRequest,
                super::OpenAiProviderError::Protocol(
                    "agent request input must be an array".to_owned(),
                ),
            )
        })?;
        let mut access_token = self.auth.access_token().await?;
        let mut response = self.send_responses(&access_token, &request).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            access_token = self.auth.force_refresh(&access_token).await?;
            response = self.send_responses(&access_token, &request).await?;
        }
        if !response.status().is_success() {
            return Err(self.http_error(response).await);
        }

        Ok(stream::agent_response_stream(
            response, model, effort, input,
        ))
    }
}
