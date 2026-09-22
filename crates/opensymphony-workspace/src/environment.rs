//! Platform-aware process environment names and explicit value precedence.
use std::collections::BTreeMap;

pub fn environment_variable_names_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
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
