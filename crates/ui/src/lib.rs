mod app;
mod components;
mod error;
mod settings;
pub mod theme;

pub use self::app::{MainServices, MainView};
pub use self::error::{
    ErrorPresentation, ErrorSeverity, MagentaError, Result, notification_for_error,
};
pub use self::settings::init as init_settings;
