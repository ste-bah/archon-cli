//! Shared provider-neutral selection rules.
use std::collections::BTreeMap;

pub fn role_model(choices: &BTreeMap<String, String>, role: &str, default: &str) -> String {
    choices
        .get(role)
        .filter(|value| !value.trim().is_empty())
        .map(String::as_str)
        .unwrap_or(default)
        .to_string()
}

pub fn endpoint(
    configured: Option<&str>,
    ambient: Option<&str>,
    allow_ambient: bool,
) -> Option<String> {
    let configured = configured.filter(|value| !value.trim().is_empty());
    let ambient = ambient.filter(|value| !value.trim().is_empty());
    (if allow_ambient {
        ambient.or(configured)
    } else {
        configured
    })
    .map(str::to_string)
}

pub fn capture_environment(
    names: impl IntoIterator<Item = String>,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    names
        .into_iter()
        .filter_map(|name| {
            lookup(&name)
                .filter(|value| !value.trim().is_empty())
                .map(|value| (name, value))
        })
        .collect()
}

/// Rates are per million tokens and belong to an exact configured model.
pub fn token_cost(rates: Option<[f64; 2]>, input: u64, output: u64) -> Option<f64> {
    if input == 0 && output == 0 {
        return Some(0.0);
    }
    let [input_rate, output_rate] = rates?;
    if ![input_rate, output_rate]
        .iter()
        .all(|rate| rate.is_finite() && *rate >= 0.0)
    {
        return None;
    }
    let cost = (input as f64 * input_rate + output as f64 * output_rate) / 1_000_000.0;
    cost.is_finite().then_some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roles_are_independent_and_keep_abstract_defaults() {
        let choices = BTreeMap::from([
            ("judge".into(), "opus".into()),
            ("reauthor".into(), "haiku".into()),
        ]);
        assert_eq!(role_model(&choices, "judge", "sonnet"), "opus");
        assert_eq!(role_model(&choices, "reauthor", "sonnet"), "haiku");
        assert_eq!(role_model(&choices, "coder", "sonnet"), "sonnet");
    }
    #[test]
    fn empty_choice_retains_default() {
        assert_eq!(
            role_model(
                &BTreeMap::from([("judge".into(), " ".into())]),
                "judge",
                "sonnet"
            ),
            "sonnet"
        );
    }
    #[test]
    fn endpoint_policy_is_independent_of_provider_name() {
        assert_eq!(
            endpoint(Some("configured"), Some("ambient"), false).as_deref(),
            Some("configured")
        );
        assert_eq!(
            endpoint(Some("configured"), Some("ambient"), true).as_deref(),
            Some("ambient")
        );
        assert_eq!(
            endpoint(Some("configured"), Some(" "), true).as_deref(),
            Some("configured")
        );
        assert_eq!(endpoint(None, Some("ambient"), false), None);
    }
    #[test]
    fn captures_custom_key_endpoint_and_auxiliary_credentials() {
        let values = BTreeMap::from([
            ("CUSTOM_KEY", "secret"),
            ("CUSTOM_URL", "endpoint"),
            ("CUSTOM_TOKEN", "token"),
            ("EMPTY", " "),
        ]);
        let names = [
            "CUSTOM_KEY",
            "CUSTOM_URL",
            "CUSTOM_TOKEN",
            "EMPTY",
            "MISSING",
        ]
        .map(str::to_string);
        let captured = capture_environment(names, |name| values.get(name).map(|v| v.to_string()));
        assert_eq!(captured.len(), 3);
        assert_eq!(captured["CUSTOM_KEY"], "secret");
        assert_eq!(captured["CUSTOM_URL"], "endpoint");
        assert_eq!(captured["CUSTOM_TOKEN"], "token");
    }
    #[test]
    fn costs_require_explicit_valid_rates() {
        assert_eq!(token_cost(None, 1, 1), None);
        assert_eq!(token_cost(None, 0, 0), Some(0.0));
        assert_eq!(
            token_cost(Some([2.0, 4.0]), 1_000_000, 1_000_000),
            Some(6.0)
        );
        for rates in [[-1.0, 2.0], [f64::NAN, 1.0], [1.0, f64::INFINITY]] {
            assert_eq!(token_cost(Some(rates), 1, 1), None);
        }
    }
}

/// Shipped exact-model prices preserve the base estimates; operators can override them.
pub fn default_model_prices() -> BTreeMap<String, [f64; 2]> {
    include!("model_prices.inc.rs")
        .into_iter()
        .map(|(model, rates)| (model.to_string(), rates))
        .collect()
}
