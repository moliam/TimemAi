use super::*;

#[test]
fn policy_is_protocol_independent_and_disable_wins() {
    for enabled in [None, Some(false), Some(true)] {
        for intensity in [
            None,
            Some("disabled"),
            Some("none"),
            Some("high"),
            Some("provider-level"),
        ] {
            let preference = ReasoningPreference::from_legacy(enabled, intensity);
            for demand in [ReasoningDemand::Ordinary, ReasoningDemand::Required] {
                let expected =
                    if enabled == Some(false) || matches!(intensity, Some("disabled" | "none")) {
                        EffectiveReasoning::Disabled
                    } else if enabled.is_none() && intensity.is_none() {
                        EffectiveReasoning::Unspecified
                    } else if demand == ReasoningDemand::Ordinary {
                        EffectiveReasoning::Disabled
                    } else {
                        EffectiveReasoning::Enabled {
                            intensity: intensity.map(str::to_owned),
                        }
                    };
                assert_eq!(preference.resolve(demand), expected);
            }
        }
    }
}

#[test]
fn daily_policy_boosts_only_with_declared_eligible_capabilities() {
    let ordered: Vec<String> = ["none", "low", "medium", "high", "max"]
        .map(str::to_owned)
        .to_vec();
    let enabled = |s: &str| EffectiveReasoning::Enabled {
        intensity: Some(s.into()),
    };
    for (daily, adaptive, critical, expected) in [
        ("low", None, false, enabled("low")),
        ("low", None, true, enabled("high")),
        ("low", Some(false), true, enabled("low")),
        ("max", Some(true), true, enabled("max")),
        ("none", None, true, EffectiveReasoning::Disabled),
        ("none", Some(true), true, enabled("high")),
    ] {
        assert_eq!(
            resolve_daily(Some(daily), false, adaptive, &ordered, critical),
            expected
        );
    }
    assert_eq!(
        resolve_daily(Some("low"), false, Some(true), &[], true),
        enabled("low")
    );
    assert_eq!(
        resolve_daily(Some("none"), false, Some(true), &["none".into()], true),
        EffectiveReasoning::Disabled
    );
    assert_eq!(
        resolve_daily(Some("none"), false, Some(true), &["high".into()], true),
        enabled("high")
    );
}
