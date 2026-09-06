use std::sync::Arc;

use magenta_core::{
    ConversationId, ConversationPage, ConversationStore, ConversationSummary, Message, MessagePage,
    MessageSequence, StorageFuture,
};

/// Application entry point for history operations, shared by the desktop views.
#[derive(Clone)]
pub struct ConversationHistory {
    store: Arc<dyn ConversationStore>,
}

impl ConversationHistory {
    #[must_use]
    pub fn new(store: Arc<dyn ConversationStore>) -> Self {
        Self { store }
    }

    #[must_use]
    pub fn initialize(&self) -> StorageFuture<()> {
        self.store.initialize()
    }

    #[must_use]
    pub fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>> {
        self.store.summaries()
    }

    #[must_use]
    pub fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage> {
        self.store.load(id)
    }

    #[must_use]
    pub fn earlier(
        &self,
        id: ConversationId,
        before: MessageSequence,
    ) -> StorageFuture<MessagePage> {
        self.store.earlier(id, before)
    }

    #[must_use]
    pub fn finalize(&self, message: Message) -> StorageFuture<()> {
        self.store.finalize(message)
    }

    #[must_use]
    pub fn delete(&self, id: ConversationId) -> StorageFuture<()> {
        self.store.delete(id)
    }

    #[must_use]
    pub fn rename(&self, id: ConversationId, title: String) -> StorageFuture<()> {
        self.store.rename(id, title)
    }

    #[must_use]
    pub fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()> {
        self.store.set_pinned(id, pinned)
    }
}
