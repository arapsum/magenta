use std::{future::Future, pin::Pin};

use super::{EffortLevel, ModelId, ProviderError, ProviderId};

pub type ModelCatalogFuture =
    Pin<Box<dyn Future<Output = Result<Vec<ModelDescriptor>, ProviderError>> + Send + 'static>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub provider: ProviderId,
    pub id: ModelId,
    pub display_name: String,
    pub description: Option<String>,
    pub priority: i32,
    pub default_effort: EffortLevel,
    pub supported_efforts: Vec<EffortLevel>,
}

pub trait ModelCatalog: Send + Sync {
    fn models(&self) -> ModelCatalogFuture;
}
