#![allow(dead_code)]

use super::*;

impl ConversationView {
    pub(super) fn begin_stream(
        &mut self,
        assistant_id: MessageId,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.streaming_message = Some(assistant_id);
        self.generation_progress = Some(GenerationProgress::new(
            assistant_id,
            provider_id.clone(),
            self.origins.get(&assistant_id).cloned(),
        ));
        self.start_generation_clock(generation, assistant_id, window, cx);
        self.generation_task = Some(cx.spawn_in(window, async move |view, window| {
            let mut completed = None;
            let mut stream = stream;
            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        _ = view.update_in(window, |view, window, cx| {
                            view.fail_stream(generation, assistant_id, &error, window, cx);
                        });
                        return;
                    }
                };
                let Ok(outcome) = view.update_in(window, |view, _, cx| {
                    view.apply_generation_event(generation, assistant_id, event, cx)
                }) else {
                    return;
                };
                if outcome.is_some() {
                    completed = outcome;
                    break;
                }
            }

            if let Some(outcome) = completed {
                _ = view.update_in(window, |view, _, cx| {
                    view.finish_stream(generation, assistant_id, outcome, cx);
                });
            } else {
                let error = ProviderError::with_kind(
                    provider_id,
                    magenta_core::ProviderErrorKind::IncompleteResponse,
                    IncompleteGeneration,
                );
                _ = view.update_in(window, |view, window, cx| {
                    view.fail_stream(generation, assistant_id, &error, window, cx);
                });
            }
        }));
        cx.emit(ConversationViewEvent::GenerationStarted);
    }

    fn apply_generation_event(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        event: GenerationEvent,
        cx: &mut Context<'_, Self>,
    ) -> Option<GenerationOutcome> {
        match event {
            GenerationEvent::Started => self.mark_provider_started(generation, assistant_id, cx),
            GenerationEvent::TextDelta(chunk) => self.push_stream_chunk_phase(
                generation,
                assistant_id,
                &chunk,
                Some(AssistantTextPhase::FinalAnswer),
                cx,
            ),
            GenerationEvent::TextDeltaWithPhase { delta, phase } => {
                self.push_stream_chunk_phase(generation, assistant_id, &delta, Some(phase), cx);
            }
            GenerationEvent::ReasoningSummaryStarted { key, title } => {
                self.update_reasoning_trace(generation, assistant_id, key, title, None, cx);
            }
            GenerationEvent::ReasoningSummaryDelta { key, delta } => {
                self.update_reasoning_trace(
                    generation,
                    assistant_id,
                    key,
                    "Thinking",
                    Some(delta),
                    cx,
                );
            }
            GenerationEvent::ReasoningSummaryCompleted { key, text } => {
                self.complete_reasoning_trace(generation, assistant_id, key, text, cx);
            }
            GenerationEvent::Completed(outcome) => return Some(outcome),
        }
        None
    }

    pub(super) fn start_generation_clock(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        self.generation_clock_task.take();
        self.generation_clock_task = Some(cx.spawn_in(window, async move |view, window| {
            loop {
                window
                    .background_executor()
                    .timer(GENERATION_CLOCK_INTERVAL)
                    .await;

                match view.update_in(window, |view, _, cx| {
                    let active = view.generation == generation
                        && view.streaming_message == Some(assistant_id)
                        && view
                            .generation_progress
                            .as_ref()
                            .is_some_and(|progress| progress.message_id == assistant_id);
                    if active {
                        cx.notify();
                    }
                    active
                }) {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
                }
            }
        }));
    }

    pub(super) fn mark_provider_started(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let Some(progress) = self
            .generation_progress
            .as_mut()
            .filter(|progress| progress.message_id == assistant_id)
        else {
            return;
        };
        if progress.provider_started_at.is_none() {
            progress.provider_started_at = Some(Instant::now());
            progress.phase = GenerationPhase::Thinking;
            tracing::debug!(
                provider = %progress.provider.0,
                message_id = assistant_id.0,
                operation = "conversation.generate",
                "provider stream started"
            );
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(super) fn push_stream_chunk(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        chunk: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.push_stream_chunk_phase(
            generation,
            assistant_id,
            chunk,
            Some(AssistantTextPhase::FinalAnswer),
            cx,
        );
    }

    pub(super) fn push_stream_chunk_phase(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        chunk: &str,
        phase: Option<AssistantTextPhase>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        if let Some(progress) = self
            .generation_progress
            .as_mut()
            .filter(|progress| progress.message_id == assistant_id)
        {
            if progress.first_text_at.is_none() {
                progress.first_text_at = Some(Instant::now());
            }
            if matches!(phase, Some(AssistantTextPhase::FinalAnswer)) {
                progress.final_answer_started = true;
                progress.phase = GenerationPhase::Responding;
            }
        }

        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.message.id == assistant_id)
        else {
            return;
        };
        let rendered = &mut self.messages[index];
        rendered.message.content.push_str(chunk);
        rendered.message.status = MessageStatus::Streaming;
        let normalized = markdown::normalize_for_text_view(&rendered.message.content);
        if let (Some(markdown), Some(previous)) = (
            rendered.markdown.as_ref(),
            rendered.markdown_source.as_ref(),
        ) {
            if let Some(delta) = normalized.strip_prefix(previous) {
                markdown.update(cx, |state, cx| state.push_str(delta, cx));
            } else {
                markdown.update(cx, |state, cx| state.set_text(&normalized, cx));
            }
        }
        rendered.markdown_source = Some(normalized);
        self.queue_math_for_messages([index], cx);
        self.list_state.remeasure_items(index..index + 1);
        cx.notify();
    }

    pub(super) fn finish_stream(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        outcome: GenerationOutcome,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let progress = self.clear_generation_progress();
        self.clear_agent_state();
        self.mark_trace_terminal(
            assistant_id,
            AssistantTraceStatus::Completed,
            progress.as_ref().map(GenerationProgress::elapsed),
        );
        if let Some(progress) = progress.as_ref() {
            trace_generation_terminal(progress, "completed");
        }

        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Complete;
            message.message.generation_outcome = Some(outcome);
        }
        self.remeasure_message(assistant_id);
        self.streaming_message = None;
        if let Some(message) = self
            .messages
            .iter()
            .find(|item| item.message.id == assistant_id)
        {
            cx.emit(ConversationViewEvent::GenerationFinished(
                message.message.clone(),
            ));
        }
        cx.notify();
    }

    pub(super) fn fail_stream(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        error: &ProviderError,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let provider = error.provider.clone();
        let progress = self.clear_generation_progress();
        self.clear_agent_state();
        self.mark_trace_terminal(
            assistant_id,
            AssistantTraceStatus::Failed,
            progress.as_ref().map(GenerationProgress::elapsed),
        );
        let model = progress
            .as_ref()
            .and_then(|progress| progress.configuration.as_ref())
            .map_or("unknown", |configuration| configuration.model.0.as_str());
        let effort = progress
            .as_ref()
            .and_then(|progress| progress.configuration.as_ref())
            .map_or("unknown", |configuration| configuration.effort.label());
        let failure = magenta_core::MessageFailure::from_provider_error(error);
        tracing::error!(
            reference_code = %failure.reference_code,
            provider = %provider.0,
            model,
            effort,
            phase = progress.as_ref().map_or("unknown", |progress| progress.phase.label()),
            elapsed_ms = progress.as_ref().map(|progress| duration_millis(progress.elapsed())),
            provider_started_ms = progress.as_ref().and_then(GenerationProgress::provider_started_ms),
            first_text_ms = progress.as_ref().and_then(GenerationProgress::first_text_ms),
            operation = "conversation.generate",
            "provider generation failed"
        );
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Failed;
            message.message.failure = Some(failure);
        }
        self.remeasure_message(assistant_id);
        self.streaming_message = None;
        if let Some(message) = self
            .messages
            .iter()
            .find(|item| item.message.id == assistant_id)
        {
            cx.emit(ConversationViewEvent::GenerationFinished(
                message.message.clone(),
            ));
        }
        cx.notify();
    }

    pub(super) fn cancel_generation(&mut self, cx: &mut Context<'_, Self>) {
        let Some(assistant_id) = self.streaming_message.take() else {
            self.generation_task.take();
            self.clear_generation_progress();
            self.clear_agent_state();
            return;
        };

        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        self.clear_agent_state();
        let progress = self.clear_generation_progress();
        self.mark_trace_terminal(
            assistant_id,
            AssistantTraceStatus::Stopped,
            progress.as_ref().map(GenerationProgress::elapsed),
        );
        if let Some(progress) = progress.as_ref() {
            trace_generation_terminal(progress, "stopped");
        }
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Stopped;
        }
        self.remeasure_message(assistant_id);
        if let Some(message) = self
            .messages
            .iter()
            .find(|item| item.message.id == assistant_id)
        {
            cx.emit(ConversationViewEvent::GenerationFinished(
                message.message.clone(),
            ));
        }
        cx.notify();
    }

    pub(crate) fn request_stop(&mut self, assistant_id: MessageId, cx: &mut Context<'_, Self>) {
        if self.generation_task.is_some() {
            self.cancel_generation(cx);
        } else if self.streaming_message == Some(assistant_id) {
            cx.emit(ConversationViewEvent::StopGeneration(assistant_id));
        }
    }

    pub(super) fn clear_generation_progress(&mut self) -> Option<GenerationProgress> {
        self.generation_clock_task.take();
        self.generation_progress.take()
    }

    pub(super) fn mark_trace_terminal(
        &mut self,
        assistant_id: MessageId,
        status: AssistantTraceStatus,
        elapsed: Option<Duration>,
    ) {
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        else {
            return;
        };
        if let Some(elapsed) = elapsed {
            message.message.assistant_trace.thinking_duration_ms = Some(duration_millis(elapsed));
        }
        let finished_at = Timestamp(chrono::Local::now().timestamp_millis());
        for entry in &mut message.message.assistant_trace.entries {
            if matches!(
                entry.status,
                AssistantTraceStatus::Streaming
                    | AssistantTraceStatus::Requested
                    | AssistantTraceStatus::Running
                    | AssistantTraceStatus::AwaitingApproval
            ) {
                entry.status = status;
                entry.finished_at = Some(finished_at);
            }
        }
    }

    fn remeasure_message(&self, message_id: MessageId) {
        if let Some(index) = self
            .messages
            .iter()
            .position(|message| message.message.id == message_id)
        {
            self.list_state.remeasure_items(index..index + 1);
        }
    }

    pub(super) fn clear_agent_state(&mut self) {
        if !self
            .agent_controller
            .as_ref()
            .is_some_and(AgentApprovalController::has_pending_workspace_review)
        {
            self.agent_controller.take();
        }
        self.pending_agent_approval.take();
        for command in self.live_commands.values_mut() {
            if command.result.is_none() {
                command.result = Some(WorkspaceCommandResult {
                    status: WorkspaceCommandStatus::Cancelled,
                    exit_code: None,
                    duration_ms: 0,
                    stdout: command.stdout.clone(),
                    stderr: command.stderr.clone(),
                    truncated: false,
                });
            }
        }
    }
}
