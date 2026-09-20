use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use magenta_core::{
    AppSettings, AppearanceMode, EffortLevel, FontChoice, GenerationPreference, GenerationSettings,
    MathFontStyle, SETTINGS_VERSION, SettingsError, SettingsFuture, SettingsStore,
    TypographySettings,
};
use parking_lot::Mutex;
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

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
    let version = integer_at(document, &["version"])
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1);
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
        version: version.max(1),
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
        generation: if version >= SETTINGS_VERSION {
            read_generation_settings(document)
        } else {
            GenerationSettings::default()
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
    write_generation(document, "chat", settings.generation.chat.as_ref());
    write_generation(document, "work", settings.generation.work.as_ref());
}

fn read_generation_settings(document: &DocumentMut) -> GenerationSettings {
    GenerationSettings {
        chat: read_generation_preference(document, "chat"),
        work: read_generation_preference(document, "work"),
    }
}

fn read_generation_preference(document: &DocumentMut, mode: &str) -> Option<GenerationPreference> {
    let mode_item = document
        .as_item()
        .get("generation")
        .and_then(|item| item.get(mode))?;
    if !mode_item.is_table_like() {
        warn_incomplete_generation(mode);
        return None;
    }

    let provider = string_at(document, &["generation", mode, "provider"]);
    let model = string_at(document, &["generation", mode, "model"]);
    let effort = string_at(document, &["generation", mode, "effort"]);
    let Some(provider) = provider.map(str::trim).filter(|value| !value.is_empty()) else {
        warn_incomplete_generation(mode);
        return None;
    };
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        warn_incomplete_generation(mode);
        return None;
    };
    let Some(effort) = effort.and_then(EffortLevel::from_wire) else {
        warn_incomplete_generation(mode);
        return None;
    };

    Some(GenerationPreference::new(
        magenta_core::ProviderId::new(provider),
        magenta_core::ModelId::new(model),
        effort,
    ))
}

fn warn_incomplete_generation(mode: &str) {
    tracing::warn!(
        mode,
        operation = "settings.load",
        "incomplete generation preference treated as Automatic"
    );
}

fn write_generation(
    document: &mut DocumentMut,
    mode: &str,
    preference: Option<&GenerationPreference>,
) {
    let Some(preference) = preference else {
        remove_generation_preference(document, mode);
        return;
    };

    let root = document
        .as_item_mut()
        .as_table_like_mut()
        .expect("settings document root should be a table");
    let generation_item = root
        .entry("generation")
        .or_insert(Item::Table(Table::new()));
    let generation = ensure_table(generation_item, "generation");
    let mode_item = generation.entry(mode).or_insert(Item::Table(Table::new()));
    let mode_table = ensure_table(mode_item, mode);
    write_generation_value(mode_table, "provider", value(preference.provider.0.clone()));
    write_generation_value(mode_table, "model", value(preference.model.0.clone()));
    write_generation_value(mode_table, "effort", value(preference.effort.wire_value()));
}

fn write_generation_value(table: &mut dyn TableLike, key: &str, replacement: Item) {
    let Some(existing) = table.get_mut(key) else {
        table.insert(key, replacement);
        return;
    };
    let decor = match existing {
        Item::Value(value) => Some(value.decor().clone()),
        Item::Table(table) => Some(table.decor().clone()),
        Item::ArrayOfTables(_) | Item::None => None,
    };
    *existing = replacement;
    if let Some(decor) = decor
        && let Some(value) = existing.as_value_mut()
    {
        *value.decor_mut() = decor;
    }
}

fn ensure_table<'a>(item: &'a mut Item, name: &str) -> &'a mut dyn TableLike {
    if !item.is_table_like() {
        *item = Item::Table(Table::new());
        tracing::warn!(
            table = name,
            operation = "settings.save",
            "replaced a non-table settings value"
        );
    }
    item.as_table_like_mut()
        .expect("table item should be available after normalization")
}

fn remove_generation_preference(document: &mut DocumentMut, mode: &str) {
    let should_remove_mode = {
        let Some(generation) = document
            .get_mut("generation")
            .and_then(Item::as_table_like_mut)
        else {
            return;
        };
        let Some(mode_item) = generation.get_mut(mode) else {
            return;
        };
        let preserved_comments = mode_item
            .as_table()
            .and_then(|table| generation_comments(table, ["provider", "model", "effort"]));
        let mode_is_empty = {
            let Some(mode_table) = mode_item.as_table_like_mut() else {
                return;
            };
            for key in ["provider", "model", "effort"] {
                mode_table.remove(key);
            }
            mode_table.is_empty()
        };
        if let Some(comments) = preserved_comments.as_deref()
            && let Some(table) = mode_item.as_table_mut()
        {
            table.decor_mut().set_prefix(comments);
        }
        mode_is_empty && preserved_comments.is_none()
    };

    let generation_has_comments = document
        .get("generation")
        .and_then(Item::as_table)
        .is_some_and(table_has_comments);
    let should_remove_generation = {
        let Some(generation) = document
            .get_mut("generation")
            .and_then(Item::as_table_like_mut)
        else {
            return;
        };
        if should_remove_mode {
            generation.remove(mode);
        }
        generation.is_empty() && !generation_has_comments
    };

    if should_remove_generation {
        document.remove("generation");
    }
}

fn generation_comments(table: &Table, keys: [&str; 3]) -> Option<String> {
    let mut comments = String::new();
    append_comments(table.decor(), &mut comments);

    for key in keys {
        if let Some(key) = table.key(key) {
            append_comments(key.leaf_decor(), &mut comments);
        }
        if let Some(item) = table.get(key) {
            match item {
                Item::Value(value) => append_comments(value.decor(), &mut comments),
                Item::Table(table) => append_comments(table.decor(), &mut comments),
                Item::ArrayOfTables(_) | Item::None => {}
            }
        }
    }

    (!comments.is_empty()).then_some(comments)
}

fn table_has_comments(table: &Table) -> bool {
    let mut comments = String::new();
    append_comments(table.decor(), &mut comments);
    !comments.is_empty()
}

fn append_comments(decor: &toml_edit::Decor, comments: &mut String) {
    for raw in [decor.prefix(), decor.suffix()].into_iter().flatten() {
        let Some(value) = raw.as_str().filter(|value| value.contains('#')) else {
            continue;
        };
        if !comments.is_empty() && !comments.ends_with('\n') {
            comments.push('\n');
        }
        comments.push_str(value);
    }
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
#[path = "../test/settings.rs"]
mod tests;
