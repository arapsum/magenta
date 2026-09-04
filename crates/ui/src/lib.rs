mod app;
mod components;
mod error;
pub mod theme;

pub use self::app::MainView;
pub use self::error::{
    ErrorPresentation, ErrorSeverity, MagentaError, Result, notification_for_error,
};
