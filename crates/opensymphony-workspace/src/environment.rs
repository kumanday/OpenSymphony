//! Platform-aware process environment names and explicit value precedence.
use std::collections::{BTreeMap, BTreeSet};

pub fn environment_variable_names_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

/// Configured targets must be distinct under the host environment's name rules.
pub(crate) fn has_environment_name_collision<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    names.into_iter().any(|name| {
        let name = if cfg!(windows) {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        };
        !seen.insert(name)
    })
}

/// Replace inherited aliases before a map is applied with `Command::envs`.
/// Its lexical iteration order must not choose which credential wins on Windows.
pub(crate) fn insert_environment_value(
    environment: &mut BTreeMap<String, String>,
    name: String,
    value: String,
) {
    environment.retain(|key, _| !environment_variable_names_equal(key, &name));
    environment.insert(name, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_environment_targets_reject_platform_equivalent_duplicates() {
        assert!(has_environment_name_collision([
            "AGENT_TOKEN",
            "AGENT_TOKEN"
        ]));
        assert_eq!(
            has_environment_name_collision(["AGENT_TOKEN", "agent_token"]),
            cfg!(windows)
        );
        assert!(!has_environment_name_collision([
            "AGENT_TOKEN",
            "OTHER_TOKEN"
        ]));
    }

    #[test]
    fn resolved_alias_wins_in_the_child_environment() {
        for inherited_name in ["ACP_TEST_ALIAS", "acp_test_alias"] {
            let mut environment = BTreeMap::from([
                (inherited_name.into(), "stale-value".into()),
                ("ACP_OTHER_VALUE".into(), "retained-value".into()),
            ]);
            insert_environment_value(
                &mut environment,
                "ACP_TEST_ALIAS".into(),
                "resolved-value".into(),
            );
            let output = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
                .arg("-c")
                .arg("import os; assert os.environ['ACP_TEST_ALIAS'] == 'resolved-value'; assert os.environ['ACP_OTHER_VALUE'] == 'retained-value'; assert os.name == 'nt' or os.environ['acp_test_alias'] == 'stale-value'")
                // Keep a distinct lowercase variable on case-sensitive hosts.
                .env("acp_test_alias", "stale-value")
                .envs(environment)
                .output()
                .expect("child environment probe");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
