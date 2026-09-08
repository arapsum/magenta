use std::sync::Arc;

use magenta_core::{
    ConversationId, ConversationPage, ConversationSearchResult, ConversationStore,
    ConversationSummary, Message, MessagePage, MessageSequence, StorageFuture,
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
    pub fn search(
        &self,
        query: String,
        limit: usize,
    ) -> StorageFuture<Vec<ConversationSearchResult>> {
        self.store.search(query, limit)
    }

    #[must_use]
    pub fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage> {
        self.store.load(id)
    }

    #[must_use]
    pub fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage> {
        self.store.load_around(id, sequence)
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
    pub fn later(&self, id: ConversationId, after: MessageSequence) -> StorageFuture<MessagePage> {
        self.store.later(id, after)
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
    pub fn rename_if_current(
        &self,
        id: ConversationId,
        current: String,
        title: String,
    ) -> StorageFuture<bool> {
        self.store.rename_if_current(id, current, title)
    }

    #[must_use]
    pub fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()> {
        self.store.set_pinned(id, pinned)
    }
}
