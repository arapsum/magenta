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
