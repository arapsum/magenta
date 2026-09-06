use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use magenta_core::{
    AppSettings, AppearanceMode, FontChoice, MathFontStyle, SETTINGS_VERSION, SettingsError,
    SettingsFuture, SettingsStore, TypographySettings,
};
use parking_lot::Mutex;
use toml_edit::{DocumentMut, value};

type Result<T> = std::result::Result<T, SettingsError>;

/// TOML-backed application settings.
///
/// The document is edited in place so comments and unknown keys remain intact
/// when a newer Magenta version writes a setting it understands.
#[derive(Clone)]
pub struct TomlSettingsStore {
    path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
}

impl TomlSettingsStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(path),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    fn load_sync(path: &Path) -> Result<AppSettings> {
        if !path.exists() {
            return Ok(AppSettings::default());
        }

        let contents = fs::read_to_string(path).map_err(SettingsError::new)?;
        let document = contents
            .parse::<DocumentMut>()
            .map_err(SettingsError::new)?;
        Ok(read_settings(&document))
    }

    fn save_sync(path: &Path, settings: &AppSettings) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(SettingsError::new)?;
        }

        let mut document = if path.exists() {
            fs::read_to_string(path)
                .map_err(SettingsError::new)?
                .parse::<DocumentMut>()
                .map_err(SettingsError::new)?
        } else {
            DocumentMut::new()
        };
        write_settings(&mut document, settings);
        write_atomically(path, document.to_string().as_bytes())
    }
}

impl SettingsStore for TomlSettingsStore {
    fn load(&self) -> SettingsFuture<AppSettings> {
        let path = Arc::clone(&self.path);
        Box::pin(smol::unblock(move || Self::load_sync(&path)))
    }

    fn save(&self, settings: AppSettings) -> SettingsFuture<()> {
        let path = Arc::clone(&self.path);
        let write_lock = Arc::clone(&self.write_lock);
        Box::pin(smol::unblock(move || {
            let _lock = write_lock.lock();
            Self::save_sync(&path, &settings)
        }))
    }

    fn reset(&self) -> SettingsFuture<AppSettings> {
        let path = Arc::clone(&self.path);
        let write_lock = Arc::clone(&self.write_lock);
        Box::pin(smol::unblock(move || {
            let _lock = write_lock.lock();
            if path.exists() {
                let backup = backup_path(&path)?;
                fs::copy(&*path, backup).map_err(SettingsError::new)?;
            }
            let settings = AppSettings::default();
            Self::save_sync(&path, &settings)?;
            Ok(settings)
        }))
    }

    fn path(&self) -> PathBuf {
        (*self.path).clone()
    }
}

fn read_settings(document: &DocumentMut) -> AppSettings {
    let defaults = AppSettings::default();
    let typography = &defaults.typography;
    let appearance =
        string_at(document, &["appearance", "theme"]).map_or(defaults.appearance, |value| {
            match value {
                "system" => AppearanceMode::System,
                "light" => AppearanceMode::Light,
                _ => AppearanceMode::Dark,
            }
        });
    let ui_font = string_at(document, &["typography", "ui_font"]).map_or_else(
        || typography.ui_font.clone(),
        |value| FontChoice::from_config_value(value, typography.ui_font.clone()),
    );
    let monospace_font = string_at(document, &["typography", "monospace_font"]).map_or_else(
        || typography.monospace_font.clone(),
        |value| FontChoice::from_config_value(value, typography.monospace_font.clone()),
    );

    AppSettings {
        version: integer_at(document, &["version"])
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(SETTINGS_VERSION),
        appearance,
        typography: TypographySettings {
            ui_font,
            ui_size: size_at(document, "ui_size", typography.ui_size),
            monospace_font,
            monospace_size: size_at(document, "monospace_size", typography.monospace_size),
            math_font: string_at(document, &["typography", "math_font"])
                .map_or(typography.math_font, MathFontStyle::from_config_value),
            inline_math_size: size_at(document, "inline_math_size", typography.inline_math_size),
            display_math_size: size_at(document, "display_math_size", typography.display_math_size),
        },
    }
}

fn write_settings(document: &mut DocumentMut, settings: &AppSettings) {
    document["version"] = value(i64::from(SETTINGS_VERSION));
    document["appearance"]["theme"] = value(match settings.appearance {
        AppearanceMode::System => "system",
        AppearanceMode::Light => "light",
        AppearanceMode::Dark => "dark",
    });
    document["typography"]["ui_font"] = value(settings.typography.ui_font.as_config_value());
    document["typography"]["ui_size"] = value(i64::from(settings.typography.ui_size));
    document["typography"]["monospace_font"] =
        value(settings.typography.monospace_font.as_config_value());
    document["typography"]["monospace_size"] = value(i64::from(settings.typography.monospace_size));
    document["typography"]["math_font"] = value(settings.typography.math_font.as_config_value());
    document["typography"]["inline_math_size"] =
        value(i64::from(settings.typography.inline_math_size));
    document["typography"]["display_math_size"] =
        value(i64::from(settings.typography.display_math_size));
}

fn string_at<'a>(document: &'a DocumentMut, path: &[&str]) -> Option<&'a str> {
    let mut item = document.as_item();
    for segment in path {
        item = item.get(segment)?;
    }
    item.as_str()
}

fn integer_at(document: &DocumentMut, path: &[&str]) -> Option<i64> {
    let mut item = document.as_item();
    for segment in path {
        item = item.get(segment)?;
    }
    item.as_integer()
}

fn size_at(document: &DocumentMut, key: &str, default: u16) -> u16 {
    integer_at(document, &["typography", key])
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (8..=72).contains(value))
        .unwrap_or(default)
}

fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = path.with_extension("toml.tmp");
    let mut file = fs::File::create(&temporary).map_err(SettingsError::new)?;
    file.write_all(contents).map_err(SettingsError::new)?;
    file.sync_all().map_err(SettingsError::new)?;
    fs::rename(temporary, path).map_err(SettingsError::new)
}

fn backup_path(path: &Path) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(SettingsError::new)?
        .as_secs();
    Ok(path.with_extension(format!("toml.bak-{timestamp}")))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn writes_settings_without_losing_unknown_keys_or_comments() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.toml");
        fs::write(
            &path,
            "# user comment\ncustom = true\n\n[appearance]\ntheme = 'light'\n",
        )
        .unwrap();

        TomlSettingsStore::save_sync(&path, &AppSettings::default()).unwrap();
        let saved = fs::read_to_string(path).unwrap();

        assert!(saved.contains("# user comment"));
        assert!(saved.contains("custom = true"));
        assert!(saved.contains("theme = \"dark\""));
    }

    #[test]
    fn missing_file_uses_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.toml");
        assert_eq!(
            TomlSettingsStore::load_sync(&path).unwrap(),
            AppSettings::default()
        );
    }
}
