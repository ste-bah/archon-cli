//! Defaults read from the shipped operator configuration, once per process.
use serde::de::DeserializeOwned;
use std::sync::OnceLock;

pub(crate) fn embedded_section<T: DeserializeOwned>(path: &[&str]) -> T {
    static TEMPLATE: OnceLock<toml::Value> = OnceLock::new();
    let template = TEMPLATE.get_or_init(|| {
        toml::from_str(&super::write_example_config())
            .expect("the shipped configuration must parse")
    });
    section(template, path)
}

fn section<T: DeserializeOwned>(template: &toml::Value, path: &[&str]) -> T {
    let mut value = template;
    for key in path {
        value = value.get(key).unwrap_or_else(|| {
            panic!(
                "shipped configuration section {} is missing",
                path.join(".")
            )
        });
    }
    value.clone().try_into().unwrap_or_else(|error| {
        panic!(
            "shipped configuration section {} is invalid: {error}",
            path.join(".")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "section absent is missing")]
    fn missing_section_names_the_shipped_key() {
        section::<Vec<String>>(&toml::Value::Table(Default::default()), &["absent"]);
    }

    #[test]
    #[should_panic(expected = "section value is invalid")]
    fn invalid_section_names_the_shipped_key() {
        let value = toml::from_str("value = 1").unwrap();
        section::<Vec<String>>(&value, &["value"]);
    }

    #[test]
    fn nested_sections_are_deserialized() {
        let value = toml::from_str("[outer]\nvalue = ['a', 'b']").unwrap();
        assert_eq!(
            section::<Vec<String>>(&value, &["outer", "value"]),
            ["a", "b"]
        );
    }
}
