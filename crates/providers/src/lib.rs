//! Provider adapters for Magenta's core chat contract.

#[cfg(test)]
mod contract;
mod demo;
mod demo_response;
mod http;
mod openai;
mod openai_auth;
mod openai_wire;
mod sse;

pub use demo::DemoProvider;
pub use openai::OpenAiProvider;
