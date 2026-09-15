//! Lazy, local sentence embeddings. The model is downloaded into the app data
//! directory on first semantic operation and is then reused offline.

use std::{path::PathBuf, sync::Arc};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use magenta_core::{AgentDataFuture, EmbeddingProvider, StorageError, StorageErrorKind};
use parking_lot::Mutex;

const DIMENSIONS: usize = 384;

#[derive(Clone)]
pub struct LocalEmbeddingProvider {
    cache_directory: Arc<PathBuf>,
    model: Arc<Mutex<Option<TextEmbedding>>>,
}

impl LocalEmbeddingProvider {
    #[must_use]
    pub fn new(cache_directory: PathBuf) -> Self {
        Self {
            cache_directory: Arc::new(cache_directory),
            model: Arc::new(Mutex::new(None)),
        }
    }
}

impl EmbeddingProvider for LocalEmbeddingProvider {
    fn dimensions(&self) -> usize {
        DIMENSIONS
    }

    fn embed(&self, texts: Vec<String>) -> AgentDataFuture<Vec<Vec<f32>>> {
        let cache_directory = Arc::clone(&self.cache_directory);
        let model = Arc::clone(&self.model);

        Box::pin(smol::unblock(move || {
            std::fs::create_dir_all(cache_directory.as_path()).map_err(super::unavailable)?;

            let mut model = model.lock();

            if model.is_none() {
                let options = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2)
                    .with_cache_dir(cache_directory.as_ref().clone())
                    .with_show_download_progress(false)
                    .with_intra_threads(2);
                *model = Some(TextEmbedding::try_new(options).map_err(embedding_error)?);
            }

            model
                .as_mut()
                .expect("embedding model initialized")
                .embed(texts, Some(32))
                .map_err(embedding_error)
        }))
    }
}

fn embedding_error(error: fastembed::Error) -> StorageError {
    StorageError::new(StorageErrorKind::Unavailable, error)
}
