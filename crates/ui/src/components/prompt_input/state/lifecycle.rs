use super::*;

impl PromptComposer {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Magenta anything…")
                .auto_grow(1, 5)
                .submit_on_enter(true)
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));
        let command_palette = cx.new(|cx| CommandState::new(window, cx));

        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |composer, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    composer.inline_error = None;
                    composer.schedule_code_preview(window, cx);
                    composer.sync_command_input(window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => composer.submit(cx),
                InputEvent::PressEnter { shift: true, .. } => {
                    composer.handle_shift_enter(window, cx);
                }
            },
        )];

        Self {
            input,
            preview,
            preview_source: String::new(),
            preview_line_count: 0,
            models: Vec::new(),
            model_catalog_state: ModelCatalogState::NotLoaded,
            model: None,
            effort: None,
            chat_generation: ModeGenerationState::default(),
            work_generation: ModeGenerationState::default(),
            generation_settings: GenerationSettings::default(),
            mode: ConversationMode::Chat,
            input_mode: ConversationMode::Chat,
            thread_state: ComposerThreadState::New,
            workspace_root: None,
            agent_capability: AgentCapability::Unavailable,
            generation_status: ComposerGenerationState::Idle,
            storage_status: ComposerReadiness::Ready,
            submission_status: ComposerReadiness::Ready,
            attachments: Vec::new(),
            chat_draft: ModeDraft::default(),
            work_draft: ModeDraft::default(),
            retry_target: None,
            inline_error: None,
            blocking_error: None,
            attachment_task: None,
            workspace_task: None,
            preview_task: None,
            preview_generation: 0,
            subscriptions,
            command_catalog: None,
            command_palette,
            command_palette_state: CommandPaletteState::Closed,
            selected_command: None,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input.focus_handle(cx).focus(window, cx);
    }

    pub fn set_configuration(
        &mut self,
        configuration: &GenerationConfig,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_persisted_generation(configuration, cx);
    }

    pub fn set_generating(&mut self, generating: bool, cx: &mut Context<'_, Self>) {
        let next = if generating {
            ComposerGenerationState::Generating
        } else {
            ComposerGenerationState::Idle
        };
        if self.generation_status != next {
            self.generation_status = next;
            cx.notify();
        }
    }

    pub(crate) fn set_thread_active(&mut self, thread_active: bool, cx: &mut Context<'_, Self>) {
        let next = if thread_active {
            ComposerThreadState::Active
        } else {
            ComposerThreadState::New
        };
        if self.thread_state != next {
            self.thread_state = next;
            cx.notify();
        }
    }

    pub(crate) const fn is_generating(&self) -> bool {
        self.generation_status.is_generating()
    }

    pub(crate) const fn thread_is_active(&self) -> bool {
        self.thread_state.is_active()
    }

    pub(crate) fn set_command_catalog(
        &mut self,
        catalog: Arc<dyn CommandCatalog>,
        cx: &mut Context<'_, Self>,
    ) {
        self.command_catalog = Some(catalog);
        cx.notify();
    }

    pub(crate) fn set_agent_capability(
        &mut self,
        capability: AgentCapability,
        cx: &mut Context<'_, Self>,
    ) {
        self.agent_capability = capability;
        if !capability.available() && self.mode == ConversationMode::Agent {
            self.mode = ConversationMode::Chat;
        }
        cx.notify();
    }

    pub(crate) fn set_conversation_context(
        &mut self,
        mode: ConversationMode,
        workspace_root: Option<PathBuf>,
        cx: &mut Context<'_, Self>,
    ) {
        self.mode = if mode == ConversationMode::Agent && !self.agent_capability.available() {
            ConversationMode::Chat
        } else {
            mode
        };
        self.workspace_root = workspace_root;
        cx.notify();
    }

    pub(crate) fn activate_workspace(&mut self, root: PathBuf, cx: &mut Context<'_, Self>) {
        if self.agent_capability.available() {
            self.mode = ConversationMode::Agent;
            self.workspace_root = Some(root);
            cx.notify();
        }
    }

    pub(crate) fn set_storage_ready(&mut self, ready: bool, cx: &mut Context<'_, Self>) {
        self.storage_status = if ready {
            ComposerReadiness::Ready
        } else {
            ComposerReadiness::NotReady
        };
        cx.notify();
    }

    pub(crate) fn set_submission_ready(&mut self, ready: bool, cx: &mut Context<'_, Self>) {
        self.submission_status = if ready {
            ComposerReadiness::Ready
        } else {
            ComposerReadiness::NotReady
        };
        cx.notify();
    }

    pub(crate) const fn storage_is_ready(&self) -> bool {
        self.storage_status.is_ready()
    }

    pub(crate) const fn submission_is_ready(&self) -> bool {
        self.submission_status.is_ready()
    }

    pub(crate) fn set_account_error(
        &mut self,
        error: Option<ErrorPresentation>,
        cx: &mut Context<'_, Self>,
    ) {
        self.blocking_error = error;
        cx.notify();
    }

    pub(crate) fn prepare_model_retry(
        &mut self,
        message_id: MessageId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.retry_target = Some(message_id);
        self.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn clear_model_retry(&mut self, cx: &mut Context<'_, Self>) {
        if self.retry_target.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn set_inline_error(
        &mut self,
        error: ErrorPresentation,
        cx: &mut Context<'_, Self>,
    ) {
        self.inline_error = Some(error);
        cx.notify();
    }

    pub(crate) const fn inline_error(&self) -> Option<ErrorPresentation> {
        self.inline_error
    }

    pub(crate) const fn blocking_error(&self) -> Option<ErrorPresentation> {
        self.blocking_error
    }

    pub(crate) const fn is_model_retry(&self) -> bool {
        self.retry_target.is_some()
    }

    pub(crate) fn prepare_continuation(
        &mut self,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.retry_target = None;
        self.input.update(cx, |input, cx| {
            input.set_value(prompt, window, cx);
        });
        self.focus(window, cx);
        cx.notify();
    }

    pub fn clear_after_submit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_task.take();
        self.clear_code_preview(cx);
        self.attachments.clear();
        self.selected_command = None;
        self.chat_draft.command_id = None;
        self.work_draft.command_id = None;
        self.inline_error = None;
        cx.notify();
    }

    pub(crate) fn clear_submitted(
        &mut self,
        submitted: &PromptRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let same_prompt = self.input.read(cx).value().trim() == submitted.prompt.as_ref();
        let same_attachments = self
            .attachments
            .iter()
            .map(|image| &image.path)
            .eq(submitted.attachments.iter());
        if same_prompt && same_attachments {
            self.clear_after_submit(window, cx);
        }
    }

    pub(crate) fn schedule_code_preview(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        self.preview_task.take();

        let source = self.input.read(cx).value().to_string();
        if !source.contains("```") {
            self.clear_code_preview(cx);
            cx.notify();
            return;
        }

        self.preview_task = Some(cx.spawn_in(window, async move |composer, window| {
            window
                .background_executor()
                .timer(Duration::from_millis(60))
                .await;
            let blocks = window
                .background_executor()
                .spawn(async move { code_fence::fenced_blocks(&source) })
                .await;

            _ = composer.update_in(window, |composer, _, cx| {
                if composer.preview_generation != generation {
                    return;
                }
                composer.preview_task = None;
                composer.apply_code_preview(&blocks, cx);
            });
        }));
        cx.notify();
    }

    fn apply_code_preview(
        &mut self,
        blocks: &[code_fence::FencedCodeBlock],
        cx: &mut Context<'_, Self>,
    ) {
        let source = code_fence::preview_markdown(blocks);
        if self.preview_source == source {
            return;
        }

        self.preview_line_count = source.lines().count();
        self.preview_source.clone_from(&source);
        self.preview
            .update(cx, |preview, cx| preview.set_text(&source, cx));
        cx.notify();
    }

    fn clear_code_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.preview_source.is_empty() {
            return;
        }
        self.preview_source.clear();
        self.preview_line_count = 0;
        self.preview
            .update(cx, |preview, cx| preview.set_text("", cx));
    }

    pub(crate) fn handle_shift_enter(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (value, cursor) = {
            let input = self.input.read(cx);
            (input.value(), input.cursor())
        };
        let Some(auto_close) = code_fence::opening_fence_after_newline(&value, cursor) else {
            self.schedule_code_preview(window, cx);
            return;
        };

        self.input.update(cx, |input, cx| {
            input.insert(auto_close.insertion, window, cx);
            input.set_selected_range(cursor..cursor, cx);
        });
        self.schedule_code_preview(window, cx);
    }
}
