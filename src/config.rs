//! Startup validation of the EFFECTIVE engine configuration (#26).
//!
//! The effective config is the project `intentumdiff.yaml` `config:` section (already
//! key-checked by Core's loader) or `--config-json`, plus the `--fuel` override. It is
//! validated before `ready` so a typo or bad value can never announce readiness and then
//! fail later, or silently fall back to defaults:
//!
//! 1. **Core's typed option validator** (`RustCoreConfig::from_json`: integer/size, range,
//!    boolean and unsupported-compatibility-override checks, including aliases). It is not
//!    exported directly, so it is reached through the public C-ABI dispatcher's
//!    `diff_python_semantic_tree` handler, which validates the config first and answers a
//!    non-Python language with a scaffold status without computing anything.
//! 2. **Keys and types Core reads but does not validate up front.** Core has no exported
//!    allowlist for the live config (its YAML loader's list is narrower than what the live
//!    handlers read), so [`ENGINE_KEYS`] mirrors the keys the live diff/review handlers
//!    consume at the pinned Core revision. This mirror is transport-side only until Core
//!    exports one effective-config validator; see the issue for the follow-up.

use intentumdiff_rust_core::c_abi;
use serde_json::{json, Map, Value};

/// Expected JSON type of a key whose type Core does not check before compute.
#[derive(Clone, Copy)]
enum Kind {
    /// Validated by Core's typed option validator (step 1).
    Core,
    Bool,
    String,
    StringList,
    Object,
}

/// Every key the live handlers read at the pinned Core revision (snake_case and the camelCase
/// aliases RustCoreConfig accepts).
const ENGINE_KEYS: &[(&str, Kind)] = &[
    // RustCoreConfig (lib.rs)
    ("min_height", Kind::Core),
    ("minHeight", Kind::Core),
    ("min_similarity", Kind::Core),
    ("minSimilarity", Kind::Core),
    ("max_nodes", Kind::Core),
    ("maxNodes", Kind::Core),
    ("max_cst_bytes", Kind::Core),
    ("maxCstBytes", Kind::Core),
    ("max_plugin_output_bytes", Kind::Core),
    ("maxPluginOutputBytes", Kind::Core),
    ("plugin_fuel", Kind::Core),
    ("pluginFuel", Kind::Core),
    ("profile_phases", Kind::Core),
    ("profilePhases", Kind::Core),
    ("collect_trace", Kind::Core),
    ("fallback_to_token_diff", Kind::Core),
    // Compatibility-only options: Core rejects any non-default value.
    ("ignore_style", Kind::Core),
    ("ignoreStyle", Kind::Core),
    ("detect_refactorings", Kind::Core),
    ("detectRefactorings", Kind::Core),
    ("leaf_text_diff", Kind::Core),
    ("leafTextDiff", Kind::Core),
    ("approx_move_threshold", Kind::Core),
    ("approxMoveThreshold", Kind::Core),
    ("extra_trivia_types", Kind::Core),
    ("extraTriviaTypes", Kind::Core),
    // live_server.rs guardrails
    ("guardrails_enabled", Kind::Bool),
    ("guardrail_policy_path", Kind::String),
    // parser_registry.rs routing
    ("strict_plugins", Kind::Bool),
    ("language_hint", Kind::String),
    ("plugin_id", Kind::String),
    ("allowed_plugins", Kind::StringList),
    // schema_context.rs
    ("schema_context", Kind::Object),
];

fn check_kind(key: &str, kind: Kind, value: &Value) -> Result<(), String> {
    let ok = match kind {
        Kind::Core => true,
        Kind::Bool => value.is_boolean(),
        Kind::String => value.is_string(),
        // Core treats null as "no allowlist".
        Kind::StringList => {
            value.is_null()
                || value
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
        }
        Kind::Object => value.is_object(),
    };
    if ok {
        return Ok(());
    }
    let expected = match kind {
        Kind::Core => unreachable!(),
        Kind::Bool => "a boolean",
        Kind::String => "a string",
        Kind::StringList => "a list of strings or null",
        Kind::Object => "an object",
    };
    Err(format!("config {key} must be {expected}, got {value}"))
}

fn check_keys(fields: &Map<String, Value>) -> Result<(), String> {
    let mut unknown: Vec<&str> = fields
        .keys()
        .map(String::as_str)
        .filter(|key| !ENGINE_KEYS.iter().any(|(known, _)| known == key))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        let mut supported: Vec<&str> = ENGINE_KEYS.iter().map(|(key, _)| *key).collect();
        supported.sort_unstable();
        return Err(format!(
            "config contains unsupported key(s): {}. Supported keys: {}",
            unknown.join(", "),
            supported.join(", ")
        ));
    }
    for (key, value) in fields {
        if let Some((_, kind)) = ENGINE_KEYS.iter().find(|(known, _)| known == key) {
            check_kind(key, *kind, value)?;
        }
    }
    Ok(())
}

/// Run Core's own typed option validator over the effective config (see module docs).
fn check_with_core(config_json: &str) -> Result<(), String> {
    let envelope = c_abi::dispatch(
        "diff_python_semantic_tree",
        &[
            json!("{}"),
            json!("{}"),
            json!(""),
            json!(""),
            // Not Python: the handler answers with a scaffold status after validating config.
            json!("intentumdiff-config-validation"),
            json!(config_json),
        ],
    );
    let envelope: Value = serde_json::from_str(&envelope)
        .map_err(|error| format!("engine config validation returned invalid JSON: {error}"))?;
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(envelope
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("engine rejected the configuration")
        .to_owned())
}

/// Validate the effective config object; `Err` names the offending key.
pub fn validate(config: &Value) -> Result<(), String> {
    let fields = config.as_object().ok_or("config must be a JSON object")?;
    check_keys(fields)?;
    check_with_core(&config.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_defaults_and_valid_values() {
        for config in [
            json!({}),
            json!({"min_similarity": 0.7, "max_nodes": 100, "plugin_fuel": -1}),
            json!({"detect_refactorings": true, "ignore_style": true, "minSimilarity": 1}),
            json!({"guardrails_enabled": false, "guardrail_policy_path": "p.yaml"}),
            json!({"strict_plugins": true, "allowed_plugins": ["python"], "language_hint": "go"}),
            json!({"allowed_plugins": null, "schema_context": {}}),
        ] {
            validate(&config).unwrap_or_else(|error| panic!("{config}: {error}"));
        }
    }

    #[test]
    fn rejects_unknown_keys_wrong_types_ranges_and_unsupported_values() {
        for (config, needle) in [
            (json!({"bogus_key": 1}), "bogus_key"),
            (json!({"ignore_styel": true}), "ignore_styel"),
            (json!({"plugin_fuel": "x"}), "plugin_fuel"),
            (json!({"pluginFuel": 1.5}), "pluginFuel"),
            (json!({"min_similarity": 2}), "min_similarity"),
            (json!({"max_nodes": -1}), "max_nodes"),
            (json!({"detect_refactorings": false}), "detect_refactorings"),
            (json!({"detectRefactorings": false}), "detectRefactorings"),
            (json!({"collect_trace": "no"}), "collect_trace"),
            (json!({"guardrails_enabled": "false"}), "guardrails_enabled"),
            (json!({"guardrail_policy_path": 7}), "guardrail_policy_path"),
            (json!({"allowed_plugins": [1]}), "allowed_plugins"),
            (json!({"schema_context": []}), "schema_context"),
        ] {
            let error = validate(&config).expect_err(&config.to_string());
            assert!(error.contains(needle), "{config}: {error}");
        }
        assert!(validate(&json!([])).is_err());
    }
}
