use gpui_component::{Icon, IconName};
use magenta_core::ProviderId;

pub fn provider_icon(provider: Option<&ProviderId>) -> Icon {
    match provider.map(|provider| provider.0.to_ascii_lowercase()) {
        Some(provider) if provider.starts_with("openai") => Icon::empty().path("icons/openai.svg"),
        Some(provider) if provider.starts_with("anthropic") => {
            Icon::empty().path("icons/anthropic.svg")
        }
        Some(provider) if provider.starts_with("claude") => Icon::empty().path("icons/claude.svg"),
        Some(provider) if provider.starts_with("qwen") => Icon::empty().path("icons/qwen.svg"),
        Some(provider) if provider.starts_with("antigravity") => {
            Icon::empty().path("icons/antigravity.svg")
        }
        Some(provider) if provider.starts_with("kimi") => Icon::empty().path("icons/kimi.svg"),
        Some(provider) if provider.starts_with("deepseek") => {
            Icon::empty().path("icons/deepseek.svg")
        }
        Some(provider) if provider.starts_with("opencode") => {
            Icon::empty().path("icons/opencode.svg")
        }
        _ => Icon::new(IconName::Bot),
    }
}
