use guac_lib::config::{agent_patch, AppConfig, SettingsPatch};
use serde_json::json;

fn patch(value: serde_json::Value) -> SettingsPatch {
    serde_json::from_value(value).unwrap()
}

#[test]
fn interleaved_partial_limit_patches_preserve_other_fields_and_last_writer_wins() {
    let mut config = AppConfig::default();
    config.apply(patch(json!({"limits": {"maxHops": 12}}))).unwrap();
    config.apply(patch(json!({"limits": {"maxStepsPerRun": 80}}))).unwrap();
    assert_eq!(config.limits.max_hops, 12);
    assert_eq!(config.limits.max_steps_per_run, 80);
    config.apply(patch(json!({"limits": {"maxHops": 10}}))).unwrap();
    assert_eq!(config.limits.max_hops, 10);
    assert_eq!(config.limits.max_steps_per_run, 80);
}

#[test]
fn old_full_patches_and_persisted_limits_remain_readable() {
    let mut config = AppConfig::default();
    let old = json!({"maxHops": 9, "maxStepsPerRun": 70, "maxFanoutPerCall": 4,
        "maxSendsPerPair": 5, "maxToolRounds": 20});
    config.apply(patch(json!({"limits": old}))).unwrap();
    assert_eq!(serde_json::to_value(config.limits).unwrap(), old);
    let saved = serde_json::to_value(&config).unwrap();
    let reloaded: AppConfig = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(serde_json::to_value(reloaded).unwrap(), saved);
}

#[test]
fn agent_limit_validation_keeps_its_previous_error() {
    let error =
        agent_patch(&json!({"limits": {"maxHops": null}}), &AppConfig::default()).unwrap_err();
    assert_eq!(error, "those values could not be read (invalid type: null, expected u16). `read` shows each setting's type.");
    let error = agent_patch(
        &json!({"limits": {"maxHops": null}, "provider": "compatible"}),
        &AppConfig::default(),
    )
    .unwrap_err();
    assert!(error.starts_with("`provider` is the operator's to change"));
    let error = agent_patch(
        &json!({"browserIdleMinutes": "bad", "limits": {"maxHops": null}}),
        &AppConfig::default(),
    )
    .unwrap_err();
    assert_eq!(error, "those values could not be read (invalid type: string \"bad\", expected u32). `read` shows each setting's type.");
}
