use super::*;

impl ConversationView {
    pub(crate) fn load_page(
        &mut self,
        loaded: magenta_core::ConversationPage,
        cx: &mut Context<'_, Self>,
    ) {
        let origins = loaded
            .page
            .messages
            .iter()
            .map(|item| (item.message.id, item.generation.clone()))
            .collect();
        self.load(
            ConversationThread {
                conversation: loaded.conversation,
                messages: loaded
                    .page
                    .messages
                    .into_iter()
                    .map(|item| item.message)
                    .collect(),
            },
            cx,
        );
        self.origins = origins;
        self.has_older = loaded.page.has_older;
        self.older_cursor = loaded.page.older_cursor;
        cx.notify();
    }

    pub(crate) fn earlier_cursor(&self) -> Option<magenta_core::MessageSequence> {
        self.has_older.then_some(self.older_cursor).flatten()
    }

    pub(crate) fn set_loading_earlier(&mut self, loading: bool, cx: &mut Context<'_, Self>) {
        self.loading_earlier = loading;
        cx.notify();
    }

    pub(crate) fn prepend_page(
        &mut self,
        page: magenta_core::MessagePage,
        cx: &mut Context<'_, Self>,
    ) {
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
            self.origins.insert(item.message.id, item.generation);
            earlier.push(Self::rendered_message(item.message, cx));
        }
        let count = earlier.len();
        earlier.append(&mut self.messages);
        self.messages = earlier;
        self.list_state.splice(0..0, count);
        anchor.item_ix += count;
        self.list_state.scroll_to(anchor);
        self.has_older = page.has_older;
        self.older_cursor = page.older_cursor;
        self.loading_earlier = false;
        self.queue_math_for_messages(0..self.messages.len(), cx);
        cx.notify();
    }

    pub(crate) fn new(
        composer: Entity<PromptComposer>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.bind_keys([gpui::KeyBinding::new(
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
            older_cursor: None,
            has_older: false,
            loading_earlier: false,
            origins: HashMap::new(),
            math_cache: Arc::new(MathCache::default()),
            math_tasks: HashMap::new(),
            attachment_preview: None,
        }
    }

    pub(crate) fn load(&mut self, thread: ConversationThread, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
        self.older_cursor = None;
        self.has_older = false;
        self.loading_earlier = false;
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
        self.cancel_generation(cx);
        self.conversation = None;
        self.messages.clear();
        self.origins.clear();
        self.older_cursor = None;
        self.has_older = false;
        self.attachment_preview = None;
        self.list_state.reset(0);
        cx.notify();
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

    pub(crate) fn start_generation(
        &mut self,
        user_message: Message,
        assistant_message: Message,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.cancel_generation(cx);
        let user = Self::rendered_message(user_message, cx);
        if let Some(conversation) = &self.conversation {
            self.origins
                .insert(assistant_message.id, conversation.generation.clone());
        }
        let assistant = Self::rendered_message(assistant_message, cx);
        let old_count = self.messages.len();
        let assistant_id = assistant.message.id;
        self.messages.push(user);
        self.messages.push(assistant);
        self.list_state.splice(old_count..old_count, 2);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn regenerate(
        &mut self,
        assistant_id: MessageId,
        assistant_message: Message,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.message.id == assistant_id)
        else {
            return;
        };

        self.cancel_generation(cx);
        if let Some(conversation) = &self.conversation {
            self.origins
                .insert(assistant_message.id, conversation.generation.clone());
        }
        let new_assistant_id = assistant_message.id;
        self.messages[index] = Self::rendered_message(assistant_message, cx);
        self.list_state.remeasure_items(index..index + 1);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(new_assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
    }

    pub(crate) fn interrupt_for_shutdown(
        &mut self,
        cx: &mut ConversationContext<'_>,
    ) -> Option<Message> {
        let id = self.streaming_message.take()?;
        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        if let Some(progress) = self.clear_generation_progress().as_ref() {
            trace_generation_terminal(progress, "interrupted");
        }
        let message = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == id)?;
        message.message.status = MessageStatus::Stopped;
        cx.notify();
        Some(message.message.clone())
    }

    pub(crate) fn request_regenerate(&self, message_id: MessageId, cx: &mut Context<'_, Self>) {
        if !self.is_streaming()
            && self
                .messages
                .iter()
                .any(|message| message.message.id == message_id)
        {
            cx.emit(ConversationViewEvent::Regenerate(message_id));
        }
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
            markdown,
            markdown_source,
            user_segments,
        }
    }

    pub(super) fn queue_math_for_messages(
        &mut self,
        indices: impl IntoIterator<Item = usize>,
        cx: &Context<'_, Self>,
    ) {
        for index in indices {
            let Some(message) = self.messages.get(index) else {
                continue;
            };
            if message.message.role != MessageRole::Assistant {
                continue;
            }
            for key in math::configured_formulas(&message.message.content, cx) {
                self.queue_math_render(key, cx);
            }
        }
    }

    fn queue_math_render(&mut self, key: FormulaKey, cx: &Context<'_, Self>) {
        if !self.math_cache.begin(key.clone()) {
            return;
        }

        let cache = self.math_cache.clone();
        let render_key = key.clone();
        let task_key = key.clone();
        self.math_tasks.insert(
            key,
            cx.spawn(async move |view, cx| {
                let result = cx
                    .background_spawn(async move { math::render_formula(&render_key) })
                    .await;
                _ = view.update(cx, |view, cx| {
                    cache.complete(task_key.clone(), result);
                    view.math_tasks.remove(&task_key);
                    view.refresh_math_messages(cx);
                });
            }),
        );
    }

    pub(crate) fn refresh_math_typography(&mut self, cx: &mut Context<'_, Self>) {
        self.math_tasks.clear();
        self.math_cache.clear();
        self.queue_math_for_messages(0..self.messages.len(), cx);
        self.refresh_math_messages(cx);
    }

    fn refresh_math_messages(&mut self, cx: &mut Context<'_, Self>) {
        for message in &mut self.messages {
            let (Some(markdown), Some(source)) = (&message.markdown, &message.markdown_source)
            else {
                continue;
            };
            markdown.update(cx, |state, cx| state.set_text(source, cx));
        }
        self.list_state.remeasure_items(0..self.messages.len());
        cx.notify();
    }
}
