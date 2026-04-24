use archon_tui::screens::settings_screen::{SettingField, SettingsScreen};

#[test]
fn test_toggle_changes_value() {
    let mut screen = SettingsScreen::new();
    screen.set_fields(vec![SettingField::Toggle {
        key: "debug".into(),
        value: false,
    }]);
    screen.toggle_selected();
    match screen.selected() {
        Some(SettingField::Toggle { value: true, .. }) => {}
        _ => panic!("toggle should set value to true"),
    }
}

#[test]
fn test_text_field_shows_key_value() {
    let mut screen = SettingsScreen::new();
    screen.set_fields(vec![SettingField::Text {
        key: "username".into(),
        value: "alice".into(),
    }]);
    let selected = screen.selected().expect("should have selection");
    match selected {
        SettingField::Text { key, value, .. } => {
            assert_eq!(key, "username");
            assert_eq!(value, "alice");
        }
        _ => panic!("expected Text field"),
    }
}

#[test]
fn test_enum_field_options() {
    let mut screen = SettingsScreen::new();
    screen.set_fields(vec![SettingField::Enum {
        key: "mode".into(),
        value: "a".into(),
        options: vec!["a".into(), "b".into(), "c".into()],
    }]);
    assert_eq!(screen.len(), 1);
}
