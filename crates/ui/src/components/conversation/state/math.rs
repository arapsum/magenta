use gpui_kit::{AppContext as _, Context};

use super::super::{ConversationView, FormulaKey};
use crate::components::math;

impl ConversationView {
    pub(super) fn reset_math(&mut self, cx: &Context<'_, Self>) {
        self.math_tasks.clear();
        self.math_cache.clear();
        self.queue_math_for_messages(0..self.messages.len(), cx);
    }

    pub(in crate::components::conversation) fn queue_math_for_messages(
        &mut self,
        indices: impl IntoIterator<Item = usize>,
        cx: &Context<'_, Self>,
    ) {
        for index in indices {
            let Some(message) = self.messages.get(index) else {
                continue;
            };
            if message.message.role != magenta_core::MessageRole::Assistant {
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
