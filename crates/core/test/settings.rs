use super::*;

#[test]
fn font_choice_preserves_known_system_values() {
    assert_eq!(
        FontChoice::from_config_value("system-ui", FontChoice::SystemMonospace),
        FontChoice::SystemUi
    );
    assert_eq!(
        FontChoice::from_config_value("Iosevka", FontChoice::SystemUi),
        FontChoice::Family("Iosevka".to_owned())
    );
}
