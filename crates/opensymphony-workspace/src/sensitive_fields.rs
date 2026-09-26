//! Shared sensitive field classification for configuration and protocol evidence.

pub(crate) fn runtime_field_is_sensitive(name: &str) -> bool {
    // Protocol/config fields use hyphens, underscores, camelCase and acronyms.
    let name = normalize_secret_field_name(name);
    [
        "access_token",
        "api_key",
        "apikey",
        "authorization",
        "access_key",
        "accesskey",
        "account_id",
        "accountid",
        "account_identifier",
        "account_identity",
        "chatgpt_account_id",
        "credential",
        "password",
        "pat",
        "secret",
        "token",
    ]
    .iter()
    .any(|part| name == *part || name.ends_with(&format!("_{part}")))
}

pub(crate) fn normalize_secret_field_name(name: &str) -> String {
    let characters = name.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(name.len());
    for (index, character) in characters.iter().copied().enumerate() {
        if character == '-' {
            if !normalized.ends_with('_') {
                normalized.push('_');
            }
            continue;
        }
        if character.is_ascii_uppercase() {
            let previous_is_lower_or_digit = characters
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous.is_ascii_lowercase() || previous.is_ascii_digit());
            let previous_is_acronym_boundary = characters
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous.is_ascii_uppercase())
                && characters
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_lowercase());
            if (previous_is_lower_or_digit || previous_is_acronym_boundary)
                && !normalized.ends_with('_')
            {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}
