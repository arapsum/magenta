mod generation;
mod math;

use super::*;

#[allow(dead_code)]
impl ConversationView {
    pub(crate) fn load_page(
        &mut self,
        loaded: magenta_core::ConversationPage,
        cx: &mut Context<'_, Self>,
    ) {
        self.reset_local_generation();
        self.live_commands.clear();
        self.trace_disclosure_overrides.clear();
        self.trace_entry_overrides.clear();
        self.conversation = Some(loaded.conversation);
        self.origins = loaded
            .page
            .messages
            .iter()
            .map(|item| (item.message.id, item.generation.clone()))
            .collect();
        self.messages = loaded
            .page
            .messages
            .into_iter()
            .map(|item| Self::rendered_stored_message(item, cx))
            .collect();
        self.has_older = loaded.page.has_older;
        self.older_cursor = loaded.page.older_cursor;
        self.has_newer = loaded.page.has_newer;
        self.newer_cursor = loaded.page.newer_cursor;
        self.page_load = PageLoadState::Idle;
        self.release_unloaded_resources(cx);
        self.list_state
            .reset_with_uniform_height(self.messages.len(), px(96.));
        self.list_state.set_follow_mode(FollowMode::Normal);
        if self.has_newer {
            self.list_state.scroll_to(gpui_kit::ListOffset {
                item_ix: self.messages.len().saturating_sub(1) / 2,
                offset_in_item: px(0.),
            });
        } else {
            self.list_state.scroll_to_end();
        }
        cx.notify();
    }

    /// Projects the coordinator's live state over the currently loaded page.
    ///
    /// The coordinator owns the stream and keeps producing snapshots while the
    /// conversation is hidden. This method only updates the render projection;
    /// it never starts or stops provider work.
    pub(crate) fn apply_live_run(&mut self, snapshot: LiveRunSnapshot, cx: &mut Context<'_, Self>) {
        if self
            .conversation
            .as_ref()
            .is_some_and(|conversation| conversation.id != snapshot.conversation.id)
        {
            return;
        }

        let old_count = self.messages.len();
        let was_streaming = self.streaming_message.is_some();
        let mut changed_indices = Vec::new();

        self.conversation = Some(snapshot.conversation);
        for live in snapshot.messages {
            self.origins
                .insert(live.message.id, live.generation.clone());
            let index = live
                .replaces
                .and_then(|id| self.messages.iter().position(|item| item.message.id == id))
                .or_else(|| {
                    self.messages
                        .iter()
                        .position(|item| item.message.id == live.message.id)
                });

            if let Some(index) = index {
                let created_at = self.messages[index].created_at;
                let sequence = live.sequence.or(self.messages[index].sequence);
                let omitted_context_messages = if live.omitted_context_messages == 0 {
                    self.messages[index].omitted_context_messages
                } else {
                    live.omitted_context_messages
                };
                let mut rendered = Self::rendered_message(live.message, cx);
                rendered.created_at = created_at;
                rendered.sequence = sequence;
                rendered.omitted_context_messages = omitted_context_messages;
                self.messages[index] = rendered;
                changed_indices.push(index);
            } else {
                let mut rendered = Self::rendered_message(live.message, cx);
                rendered.sequence = live.sequence;
                rendered.created_at = live.created_at;
                rendered.omitted_context_messages = live.omitted_context_messages;
                self.messages.push(rendered);
            }
        }

        self.streaming_message = snapshot.streaming_message;
        self.generation_progress = snapshot.generation_progress;
        self.generation_clock_task.take();
        self.agent_controller = snapshot.agent_controller;
        self.pending_agent_approval = snapshot.pending_agent_approval;
        self.live_commands = snapshot.live_commands;
        self.queue_math_for_messages(0..self.messages.len(), cx);

        if self.messages.len() > old_count {
            self.list_state
                .splice(old_count..old_count, self.messages.len() - old_count);
            self.list_state
                .remeasure_items(old_count..self.messages.len());
        }
        changed_indices.sort_unstable();
        changed_indices.dedup();
        for index in changed_indices {
            self.list_state.remeasure_items(index..index + 1);
        }

        self.list_state
            .set_follow_mode(if self.streaming_message.is_some() {
                FollowMode::Tail
            } else {
                FollowMode::Normal
            });
        if self.streaming_message.is_some() && !was_streaming {
            self.list_state.scroll_to_end();
        }
        cx.notify();
    }

    pub(crate) fn earlier_cursor(&self) -> Option<magenta_core::MessageSequence> {
        self.has_older.then_some(self.older_cursor).flatten()
    }

    pub(crate) fn later_cursor(&self) -> Option<magenta_core::MessageSequence> {
        self.has_newer.then_some(self.newer_cursor).flatten()
    }

    pub(crate) const fn is_viewing_older_messages(&self) -> bool {
        self.has_newer
    }

    pub(crate) fn scroll_to_message(&self, id: MessageId, cx: &mut Context<'_, Self>) {
        if let Some(item_ix) = self
            .messages
            .iter()
            .position(|rendered| rendered.message.id == id)
        {
            self.list_state.scroll_to(gpui_kit::ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }

    pub(crate) fn set_loading_earlier(&mut self, loading: bool, cx: &mut Context<'_, Self>) {
        self.page_load = if loading {
            PageLoadState::Earlier
        } else {
            PageLoadState::Idle
        };
        cx.notify();
    }

    pub(crate) fn set_loading_newer(&mut self, loading: bool, cx: &mut Context<'_, Self>) {
        self.page_load = if loading {
            PageLoadState::Newer
        } else {
            PageLoadState::Idle
        };
        cx.notify();
    }

    pub(crate) fn prepend_page(
        &mut self,
        page: magenta_core::MessagePage,
        cx: &mut Context<'_, Self>,
    ) {
        let had_newer = self.has_newer;
        let mut anchor = self.list_state.logical_scroll_top();
        let mut earlier = Vec::new();
        for item in page.messages {
            if self
                .messages
                .iter()
                .any(|loaded| loaded.message.id == item.message.id)
            {
                continue;
            }
            self.origins
                .insert(item.message.id, item.generation.clone());
            earlier.push(Self::rendered_stored_message(item, cx));
        }
        let count = earlier.len();
        earlier.append(&mut self.messages);
        self.messages = earlier;
        let overflow = self.messages.len().saturating_sub(MAX_RENDERED_MESSAGES);
        if overflow > 0 {
            self.messages.truncate(MAX_RENDERED_MESSAGES);
        }
        self.release_unloaded_resources(cx);
        self.list_state
            .reset_with_uniform_height(self.messages.len(), px(96.));
        anchor.item_ix += count;
        anchor.item_ix = anchor.item_ix.min(self.messages.len().saturating_sub(1));
        self.list_state.scroll_to(anchor);
        self.has_older = page.has_older;
        self.older_cursor = page.older_cursor;
        self.has_newer = had_newer || overflow > 0;
        self.newer_cursor = self.messages.last().and_then(|message| message.sequence);
        self.page_load = PageLoadState::Idle;
        cx.notify();
    }

    pub(crate) fn append_page(
        &mut self,
        page: magenta_core::MessagePage,
        cx: &mut Context<'_, Self>,
    ) {
        let had_older = self.has_older;
        let mut anchor = self.list_state.logical_scroll_top();
        for item in page.messages {
            if self
                .messages
                .iter()
                .any(|loaded| loaded.message.id == item.message.id)
            {
                continue;
            }
            self.origins
                .insert(item.message.id, item.generation.clone());
            self.messages.push(Self::rendered_stored_message(item, cx));
        }
        let overflow = self.messages.len().saturating_sub(MAX_RENDERED_MESSAGES);
        if overflow > 0 {
            self.messages.drain(..overflow);
            anchor.item_ix = anchor.item_ix.saturating_sub(overflow);
        }
        self.release_unloaded_resources(cx);
        self.list_state
            .reset_with_uniform_height(self.messages.len(), px(96.));
        self.list_state.scroll_to(anchor);
        self.has_older = had_older || overflow > 0;
        self.older_cursor = self.messages.first().and_then(|message| message.sequence);
        self.has_newer = page.has_newer;
        self.newer_cursor = page.newer_cursor;
        self.page_load = PageLoadState::Idle;
        cx.notify();
    }

    pub(crate) fn new(
        composer: Entity<PromptComposer>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "escape",
            CloseAttachmentPreview,
            Some("AttachmentPreview"),
        )]);
        Self {
            composer,
            conversation: None,
            messages: Vec::new(),
            list_state: ListState::new(0, ListAlignment::Top, LIST_OVERDRAW)
                .with_uniform_item_height(px(96.)),
            generation: 0,
            streaming_message: None,
            generation_task: None,
            generation_clock_task: None,
            generation_progress: None,
            agent_controller: None,
            pending_agent_approval: None,
            live_commands: HashMap::new(),
            trace_disclosure_overrides: HashMap::new(),
            trace_entry_overrides: HashMap::new(),
            older_cursor: None,
            has_older: false,
            page_load: PageLoadState::Idle,
            newer_cursor: None,
            has_newer: false,
            origins: HashMap::new(),
            math_cache: Arc::new(MathCache::default()),
            math_tasks: HashMap::new(),
            attachment_preview: None,
            attachment_preview_focus: cx.focus_handle(),
            attachment_preview_return_focus: None,
        }
    }

    pub(crate) fn load(&mut self, thread: ConversationThread, cx: &mut Context<'_, Self>) {
        self.reset_local_generation();
        self.live_commands.clear();
        self.trace_disclosure_overrides.clear();
        self.trace_entry_overrides.clear();
        self.older_cursor = None;
        self.has_older = false;
        self.page_load = PageLoadState::Idle;
        self.newer_cursor = None;
        self.has_newer = false;
        self.origins.clear();
        self.conversation = Some(thread.conversation);
        self.messages = thread
            .messages
            .into_iter()
            .map(|message| Self::rendered_message(message, cx))
            .collect();
        self.queue_math_for_messages(0..self.messages.len(), cx);
        self.list_state
            .reset_with_uniform_height(self.messages.len(), px(96.));
        self.list_state.set_follow_mode(FollowMode::Normal);
        self.list_state.scroll_to_end();
        cx.notify();
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<'_, Self>) {
        self.reset_local_generation();
        self.live_commands.clear();
        self.trace_disclosure_overrides.clear();
        self.trace_entry_overrides.clear();
        self.conversation = None;
        self.messages.clear();
        self.origins.clear();
        self.older_cursor = None;
        self.has_older = false;
        self.newer_cursor = None;
        self.has_newer = false;
        self.page_load = PageLoadState::Idle;
        self.attachment_preview = None;
        self.attachment_preview_return_focus = None;
        self.list_state.reset(0);
        cx.notify();
    }

    fn reset_local_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        self.generation_clock_task.take();
        self.streaming_message = None;
        self.generation_progress = None;
        self.agent_controller = None;
        self.pending_agent_approval = None;
        self.live_commands.clear();
    }

    pub(crate) fn set_generation(
        &mut self,
        generation: GenerationConfig,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(conversation) = &mut self.conversation {
            conversation.generation = generation;
            cx.notify();
        }
    }

    pub(crate) fn rename(
        &mut self,
        id: magenta_core::ConversationId,
        title: String,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(conversation) = &mut self.conversation
            && conversation.id == id
        {
            conversation.title = title;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Option<ConversationThread> {
        Some(ConversationThread {
            conversation: self.conversation.clone()?,
            messages: self
                .messages
                .iter()
                .map(|message| message.message.clone())
                .collect(),
        })
    }

    pub(crate) const fn is_streaming(&self) -> bool {
        self.streaming_message.is_some()
    }

    pub(crate) fn request_regenerate(&self, message_id: MessageId, cx: &mut Context<'_, Self>) {
        if !self.is_streaming()
            && self.messages.iter().any(|message| {
                message.message.id == message_id
                    && message.message.status != MessageStatus::Streaming
            })
        {
            let Some(message) = self
                .messages
                .iter()
                .find(|message| message.message.id == message_id)
            else {
                return;
            };
            let failed = message.message.status == MessageStatus::Failed;
            let has_side_effects = message.message.assistant_trace.entries.iter().any(|entry| {
                entry.kind == AssistantTraceKind::Tool
                    && matches!(
                        entry.tool_name.as_deref(),
                        Some("create_file" | "apply_patch" | "run_command")
                    )
            });
            cx.emit(if has_side_effects && failed {
                ConversationViewEvent::PrepareContinue(message_id)
            } else if failed {
                ConversationViewEvent::Retry(message_id)
            } else {
                ConversationViewEvent::Regenerate(message_id)
            });
        }
    }

    pub(crate) fn request_choose_model(&self, message_id: MessageId, cx: &mut Context<'_, Self>) {
        if !self.is_streaming() {
            cx.emit(ConversationViewEvent::ChooseModelForRetry(message_id));
        }
    }

    pub(crate) fn generation_for_message(
        &self,
        message_id: MessageId,
    ) -> Option<magenta_core::GenerationConfig> {
        self.origins.get(&message_id).cloned()
    }

    pub(crate) fn conversation_generation(&self) -> Option<magenta_core::GenerationConfig> {
        self.conversation
            .as_ref()
            .map(|conversation| conversation.generation.clone())
    }

    pub(crate) fn conversation_details(&self) -> Option<Conversation> {
        self.conversation.clone()
    }

    pub(crate) fn is_agent_conversation(&self) -> bool {
        self.conversation
            .as_ref()
            .is_some_and(|conversation| conversation.mode == magenta_core::ConversationMode::Agent)
    }

    pub(super) fn rendered_message(
        message: Message,
        cx: &mut Context<'_, Self>,
    ) -> RenderedMessage {
        let (markdown, markdown_source, user_segments) = match message.role {
            MessageRole::Assistant => {
                let source = markdown::normalize_for_text_view(&message.content);
                (
                    Some(cx.new(|cx| TextViewState::markdown(&source, cx))),
                    Some(source),
                    Vec::new(),
                )
            }
            MessageRole::User => (
                None,
                None,
                code_fence::parse_segments(&message.content)
                    .into_iter()
                    .map(|segment| match segment {
                        ContentSegment::Text(text) => RenderedUserSegment::Text(text),
                        ContentSegment::Code(block) => {
                            let source_start = block.source_start;
                            let markdown = code_fence::markdown_for_block(&block);
                            RenderedUserSegment::Code {
                                source_start,
                                markdown: cx.new(|cx| TextViewState::markdown(&markdown, cx)),
                            }
                        }
                    })
                    .collect(),
            ),
        };
        RenderedMessage {
            message,
            created_at: magenta_core::Timestamp(chrono::Local::now().timestamp_millis()),
            markdown,
            markdown_source,
            user_segments,
            sequence: None,
            omitted_context_messages: 0,
        }
    }

    fn rendered_stored_message(
        item: magenta_core::StoredMessage,
        cx: &mut Context<'_, Self>,
    ) -> RenderedMessage {
        let sequence = item.sequence;
        let created_at = item.created_at;
        let omitted_context_messages = item.omitted_context_messages;
        let mut rendered = Self::rendered_message(item.message, cx);
        rendered.sequence = Some(sequence);
        rendered.created_at = created_at;
        rendered.omitted_context_messages = omitted_context_messages;
        rendered
    }

    fn release_unloaded_resources(&mut self, cx: &Context<'_, Self>) {
        self.origins.retain(|id, _| {
            self.messages
                .iter()
                .any(|message| message.message.id == *id)
        });
        self.trace_entry_overrides.retain(|(message_id, _), _| {
            self.messages
                .iter()
                .any(|message| message.message.id == *message_id)
        });
        self.trace_disclosure_overrides.retain(|message_id, _| {
            self.messages
                .iter()
                .any(|message| message.message.id == *message_id)
        });
        self.reset_math(cx);
    }

    pub(super) fn trim_oldest_to_limit(&mut self, cx: &Context<'_, Self>) -> usize {
        let overflow = self.messages.len().saturating_sub(MAX_RENDERED_MESSAGES);
        if overflow == 0 {
            return 0;
        }
        self.messages.drain(..overflow);
        self.has_older = true;
        self.older_cursor = self.messages.first().and_then(|message| message.sequence);
        self.release_unloaded_resources(cx);
        overflow
    }
}
