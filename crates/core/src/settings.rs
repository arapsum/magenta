use std::{future::Future, path::PathBuf, pin::Pin};

use serde::{Deserialize, Serialize};

use crate::{ConversationMode, EffortLevel, ModelId, ProviderId};

/// The persisted settings format supported by this version of Magenta.
pub const SETTINGS_VERSION: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppearanceMode {
    System,
    Light,
    #[default]
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontChoice {
    SystemUi,
    SystemMonospace,
    Family(String),
}

impl FontChoice {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::SystemUi => "System UI".to_owned(),
            Self::SystemMonospace => "System monospace".to_owned(),
            Self::Family(name) => name.clone(),
        }
    }

    #[must_use]
    pub fn as_config_value(&self) -> String {
        match self {
            Self::SystemUi => "system-ui".to_owned(),
            Self::SystemMonospace => "system-monospace".to_owned(),
            Self::Family(name) => name.clone(),
        }
    }

    #[must_use]
    pub fn from_config_value(value: &str, default: Self) -> Self {
        match value.trim() {
            "system-ui" => Self::SystemUi,
            "system-monospace" => Self::SystemMonospace,
            "" => default,
            name => Self::Family(name.to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MathFontStyle {
    #[default]
    Default,
    Roman,
    SansSerif,
    Typewriter,
}

impl MathFontStyle {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "KaTeX Default",
            Self::Roman => "KaTeX Roman",
            Self::SansSerif => "KaTeX Sans-serif",
            Self::Typewriter => "KaTeX Typewriter",
        }
    }

    #[must_use]
    pub const fn as_config_value(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Roman => "roman",
            Self::SansSerif => "sans-serif",
            Self::Typewriter => "typewriter",
        }
    }

    #[must_use]
    pub fn from_config_value(value: &str) -> Self {
        match value {
            "roman" => Self::Roman,
            "sans-serif" => Self::SansSerif,
            "typewriter" => Self::Typewriter,
            _ => Self::Default,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypographySettings {
    pub ui_font: FontChoice,
    pub ui_size: u16,
    pub monospace_font: FontChoice,
    pub monospace_size: u16,
    pub math_font: MathFontStyle,
    pub inline_math_size: u16,
    pub display_math_size: u16,
}

impl Default for TypographySettings {
    fn default() -> Self {
        Self {
            ui_font: FontChoice::SystemUi,
            ui_size: 15,
            monospace_font: FontChoice::SystemMonospace,
            monospace_size: 13,
            math_font: MathFontStyle::Default,
            inline_math_size: 13,
            display_math_size: 16,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationPreference {
    pub provider: ProviderId,
    pub model: ModelId,
    pub effort: EffortLevel,
}

impl GenerationPreference {
    #[must_use]
    pub const fn new(provider: ProviderId, model: ModelId, effort: EffortLevel) -> Self {
        Self {
            provider,
            model,
            effort,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationSettings {
    pub chat: Option<GenerationPreference>,
    pub work: Option<GenerationPreference>,
}

impl GenerationSettings {
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub const fn for_mode(&self, mode: ConversationMode) -> Option<&GenerationPreference> {
        match mode {
            ConversationMode::Chat => self.chat.as_ref(),
            ConversationMode::Agent => self.work.as_ref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub version: u32,
    pub appearance: AppearanceMode,
    pub typography: TypographySettings,
    pub generation: GenerationSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            appearance: AppearanceMode::default(),
            typography: TypographySettings::default(),
            generation: GenerationSettings::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("settings operation failed")]
pub struct SettingsError {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
}

impl SettingsError {
    #[must_use]
    pub fn new(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

pub type SettingsFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, SettingsError>> + Send + 'static>>;

pub trait SettingsStore: Send + Sync {
    fn load(&self) -> SettingsFuture<AppSettings>;

    fn save(&self, settings: AppSettings) -> SettingsFuture<()>;

    fn reset(&self) -> SettingsFuture<AppSettings>;

    fn path(&self) -> PathBuf;
}

#[cfg(test)]
#[path = "../test/settings.rs"]
mod tests;
