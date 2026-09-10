//! Provider adapters for Magenta's chat, agent, account, and model contracts.
//!
//! [`OpenAiProvider`] implements `ChatGPT` browser authentication, model discovery,
//! chat streaming, and agent protocol continuations. [`DemoProvider`] supplies
//! deterministic chat streams for tests. HTTP payloads and credential handling
//! stay here; conversation persistence and tool execution belong to callers.

#[cfg(test)]
mod contract;
mod demo;
mod demo_response;
mod openai;

pub use demo::DemoProvider;
pub use openai::OpenAiProvider;
