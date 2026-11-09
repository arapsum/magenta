//! Native GPUI views, interaction state, and error presentation for Magenta.
//!
//! [`MainView`] receives application workflows and injected [`MainServices`],
//! owns stream and view-task lifetimes, and coordinates conversation recovery.
//! [`init_settings`] and [`theme`] initialize appearance before view construction.
//! Concrete provider, database, and workspace adapters are wired by desktop.

mod app;
mod components;
mod error;
mod settings;
pub mod theme;

// Keep the extra interface icons explicit so the full Lucide catalog is not embedded.
gpui_kit::assets::icon_assets!(pub UiIcons, [GitBranch, Lightbulb, ListTodo, MessageCircle]);

pub use self::app::{BubblewrapCapability, MainServices, MainView, WorkRuntimeReadiness};
pub use self::error::{
    ErrorPresentation, ErrorSeverity, MagentaError, Result, notification_for_error,
    provider_error_presentation,
};
pub use self::settings::init as init_settings;
