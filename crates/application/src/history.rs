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

    pub fn initialize(&self) -> StorageFuture<()> {
        self.store.initialize()
    }

    pub fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>> {
        self.store.summaries()
    }

    pub fn search(
        &self,
        query: String,
        limit: usize,
    ) -> StorageFuture<Vec<ConversationSearchResult>> {
        self.store.search(query, limit)
    }

    pub fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage> {
        self.store.load(id)
    }

    pub fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage> {
        self.store.load_around(id, sequence)
    }

    pub fn earlier(
        &self,
        id: ConversationId,
        before: MessageSequence,
    ) -> StorageFuture<MessagePage> {
        self.store.earlier(id, before)
    }

    pub fn later(&self, id: ConversationId, after: MessageSequence) -> StorageFuture<MessagePage> {
        self.store.later(id, after)
    }

    pub fn finalize(&self, message: Message) -> StorageFuture<()> {
        self.store.finalize(message)
    }

    pub fn delete(&self, id: ConversationId) -> StorageFuture<()> {
        self.store.delete(id)
    }

    pub fn rename(&self, id: ConversationId, title: String) -> StorageFuture<()> {
        self.store.rename(id, title)
    }

    pub fn rename_if_current(
        &self,
        id: ConversationId,
        current: String,
        title: String,
    ) -> StorageFuture<bool> {
        self.store.rename_if_current(id, current, title)
    }

    pub fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()> {
        self.store.set_pinned(id, pinned)
    }
}
