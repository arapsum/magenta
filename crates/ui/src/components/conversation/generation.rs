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
                match event {
                    Ok(GenerationEvent::Started) => {
                        if view
                            .update_in(window, |view, _, cx| {
                                view.mark_provider_started(generation, assistant_id, cx);
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(GenerationEvent::TextDelta(chunk)) => {
                        if view
                            .update_in(window, |view, _, cx| {
                                view.push_stream_chunk(generation, assistant_id, &chunk, cx);
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(GenerationEvent::Completed(outcome)) => {
                        completed = Some(outcome);
                        break;
                    }
                    Err(error) => {
                        _ = view.update_in(window, |view, window, cx| {
                            view.fail_stream(generation, assistant_id, error, window, cx);
                        });
                        return;
                    }
                }
            }

            if let Some(outcome) = completed {
                _ = view.update_in(window, |view, _, cx| {
                    view.finish_stream(generation, assistant_id, outcome, cx);
                });
            } else {
                let error = ProviderError::new(provider_id, IncompleteGeneration);
                _ = view.update_in(window, |view, window, cx| {
                    view.fail_stream(generation, assistant_id, error, window, cx);
                });
            }
        }));
        cx.emit(ConversationViewEvent::GenerationStarted);
    }

    fn start_generation_clock(
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

    pub(super) fn push_stream_chunk(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        chunk: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        if let Some(progress) = self
            .generation_progress
            .as_mut()
            .filter(|progress| progress.message_id == assistant_id)
            && progress.first_text_at.is_none()
        {
            progress.first_text_at = Some(Instant::now());
            progress.phase = GenerationPhase::Responding;
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

    fn finish_stream(
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

    fn fail_stream(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        error: ProviderError,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let provider = error.provider.clone();
        let progress = self.clear_generation_progress();
        let model = progress
            .as_ref()
            .and_then(|progress| progress.configuration.as_ref())
            .map_or("unknown", |configuration| configuration.model.0.as_str());
        let effort = progress
            .as_ref()
            .and_then(|progress| progress.configuration.as_ref())
            .map_or("unknown", |configuration| configuration.effort.label());
        tracing::error!(
            error = ?error,
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
        }
        self.streaming_message = None;
        let application_error = MagentaError::ProviderGeneration {
            provider,
            source: error,
        };
        window.push_notification(notification_for_error(&application_error), cx);
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
            return;
        };

        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        if let Some(progress) = self.clear_generation_progress().as_ref() {
            trace_generation_terminal(progress, "stopped");
        }
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Stopped;
        }
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

    pub(super) fn clear_generation_progress(&mut self) -> Option<GenerationProgress> {
        self.generation_clock_task.take();
        self.generation_progress.take()
    }
}
