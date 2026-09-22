use std::fs;

use magenta_core::{ModelId, ProviderId};

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

#[test]
fn version_one_ignores_generation_tables_until_an_explicit_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    fs::write(
        &path,
        "version = 1\n\n[generation.chat]\nprovider = \"openai\"\nmodel = \"legacy\"\neffort = \"high\"\n",
    )
    .unwrap();

    let loaded = TomlSettingsStore::load_sync(&path).unwrap();
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.generation, GenerationSettings::default());
    assert!(fs::read_to_string(&path).unwrap().contains("version = 1"));

    TomlSettingsStore::save_sync(&path, &loaded).unwrap();
    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("version = 3"));
    assert!(!saved.contains("model = \"legacy\""));
}

#[test]
fn version_two_preserves_generation_and_requires_setup_acknowledgement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    fs::write(
        &path,
        "version = 2\n\n[generation.chat]\nprovider = \"openai\"\nmodel = \"gpt-5.4\"\neffort = \"high\"\n",
    )
    .unwrap();

    let loaded = TomlSettingsStore::load_sync(&path).unwrap();
    assert_eq!(loaded.version, 2);
    assert!(loaded.generation.chat.is_some());
    assert!(!loaded.onboarding.is_setup_acknowledged());
}

#[test]
fn onboarding_acknowledgement_round_trips_in_version_three() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    let mut settings = AppSettings::default();
    settings.onboarding.set_setup_acknowledged(true);

    TomlSettingsStore::save_sync(&path, &settings).unwrap();
    let loaded = TomlSettingsStore::load_sync(&path).unwrap();

    assert!(loaded.onboarding.is_setup_acknowledged());
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains("setup_acknowledged = true")
    );
}

#[test]
fn generation_preferences_round_trip_independently_with_custom_effort() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    let mut settings = AppSettings::default();
    settings.generation.chat = Some(GenerationPreference::new(
        ProviderId::new("openai"),
        ModelId::new("gpt-5.4"),
        EffortLevel::Medium,
    ));
    settings.generation.work = Some(GenerationPreference::new(
        ProviderId::new("openai"),
        ModelId::new("gpt-5.6-codex"),
        EffortLevel::from_wire("thinking_budget").unwrap(),
    ));

    TomlSettingsStore::save_sync(&path, &settings).unwrap();
    let loaded = TomlSettingsStore::load_sync(&path).unwrap();
    assert_eq!(loaded.generation, settings.generation);
}

#[test]
fn incomplete_generation_table_is_automatic_and_preserves_source_until_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    fs::write(
        &path,
        "version = 2\n\n[generation.chat]\nprovider = \"openai\"\ncommentary = true\n",
    )
    .unwrap();

    let loaded = TomlSettingsStore::load_sync(&path).unwrap();
    assert_eq!(loaded.generation.chat, None);
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains("commentary = true"));

    TomlSettingsStore::save_sync(&path, &loaded).unwrap();
    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("commentary = true"));
    assert!(!saved.contains("provider = \"openai\""));
}

#[test]
fn automatic_removes_only_recognized_keys_and_retains_unrelated_mode_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    fs::write(
        &path,
        "version = 2\n\n[generation.chat]\nprovider = \"openai\"\nmodel = \"gpt-5.4\"\neffort = \"medium\"\ncommentary = true\n\n[generation.work]\nprovider = \"openai\"\nmodel = \"gpt-5.6-codex\"\neffort = \"high\"\n",
    )
    .unwrap();

    let mut settings = TomlSettingsStore::load_sync(&path).unwrap();
    settings.generation.chat = None;
    settings.generation.work = None;
    TomlSettingsStore::save_sync(&path, &settings).unwrap();
    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("commentary = true"));
    assert!(!saved.contains("gpt-5.4"));
    assert!(!saved.contains("gpt-5.6-codex"));
    assert!(!saved.contains("[generation.work]"));
}

#[test]
fn automatic_keeps_comments_when_a_mode_has_no_unrelated_keys() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    fs::write(
        &path,
        "version = 2\n\n[generation.chat]\n# Keep this note\nprovider = \"openai\"\nmodel = \"gpt-5.4\"\neffort = \"medium\"\n",
    )
    .unwrap();

    let mut settings = TomlSettingsStore::load_sync(&path).unwrap();
    settings.generation.chat = None;
    TomlSettingsStore::save_sync(&path, &settings).unwrap();
    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("# Keep this note"));
}

#[test]
fn configured_generation_save_preserves_existing_comments() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    fs::write(
        &path,
        "version = 2\n\n[generation.chat]\n# Keep this note\nprovider = \"openai\" # Keep provider context\nmodel = \"old-model\"\neffort = \"medium\"\n",
    )
    .unwrap();

    let mut settings = TomlSettingsStore::load_sync(&path).unwrap();
    settings.generation.chat = Some(GenerationPreference::new(
        ProviderId::new("openai"),
        ModelId::new("new-model"),
        EffortLevel::High,
    ));
    TomlSettingsStore::save_sync(&path, &settings).unwrap();
    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("# Keep this note"));
    assert!(saved.contains("# Keep provider context"));
    assert!(saved.contains("model = \"new-model\""));
}
