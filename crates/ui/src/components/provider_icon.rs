use gpui_component::{Icon, IconName};
use magenta_core::ProviderId;

pub fn provider_icon(provider: Option<&ProviderId>) -> Icon {
    if provider.is_some_and(|provider| provider.0.eq_ignore_ascii_case("openai")) {
        Icon::empty().path("icons/openai.svg")
    } else {
        Icon::new(IconName::Bot)
    }
}
