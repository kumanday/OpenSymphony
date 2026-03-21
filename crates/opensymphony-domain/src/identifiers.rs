use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentifierError {
    #[error("{kind} cannot be empty")]
    Empty { kind: &'static str },
}

macro_rules! string_identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdentifierError::Empty {
                        kind: stringify!($name),
                    });
                }

                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

string_identifier!(ConversationId);
string_identifier!(IssueId);
string_identifier!(IssueIdentifier);
string_identifier!(TrackerStateId);
string_identifier!(WorkspaceKey);
string_identifier!(WorkerId);
