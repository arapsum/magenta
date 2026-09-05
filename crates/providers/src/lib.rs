//! Provider adapters for Magenta's core chat contract.

#[cfg(test)]
mod contract;
mod demo;
mod demo_response;
mod openai;

pub use demo::DemoProvider;
pub use openai::OpenAiProvider;
