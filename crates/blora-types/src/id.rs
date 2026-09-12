// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::BloraError;

macro_rules! prefixed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            #[must_use]
            pub fn generate() -> Self {
                let uuid = uuid::Uuid::now_v7();
                Self(format!("{}_{uuid}", $prefix, uuid = uuid.simple()))
            }

            pub fn parse(value: impl AsRef<str>) -> Result<Self, BloraError> {
                value.as_ref().parse()
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = BloraError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let expected = concat!($prefix, "_");
                if !s.starts_with(expected) || s.len() <= expected.len() {
                    return Err(BloraError::InvalidId(format!(
                        "expected {expected}…, got {s}"
                    )));
                }
                Ok(Self(s.to_owned()))
            }
        }
    };
}

prefixed_id!(SessionId, "ses");
prefixed_id!(RunId, "run");
prefixed_id!(TurnId, "trn");
prefixed_id!(EventId, "evt");
prefixed_id!(TaskId, "tsk");
prefixed_id!(AgentId, "agt");
prefixed_id!(WorkspaceId, "wsp");
prefixed_id!(ApprovalId, "apr");
prefixed_id!(ArtifactId, "art");
prefixed_id!(CheckpointId, "chk");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_parses_prefixed_ids() {
        let session = SessionId::generate();
        assert!(session.as_str().starts_with("ses_"));
        assert_eq!(SessionId::parse(session.as_str()).unwrap(), session);
        assert!(EventId::parse("evt").is_err());
        assert!(RunId::parse("ses_abc").is_err());
    }
}
