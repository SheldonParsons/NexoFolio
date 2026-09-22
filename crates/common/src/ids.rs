use std::{fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! identifier {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self { Self(Uuid::new_v4()) }
        }
        impl Default for $name {
            fn default() -> Self { Self::new() }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    )+};
}

identifier!(ProjectId, EnvironmentId, UserId, TokenId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_and_reject_invalid_input() {
        let id = ProjectId::new();
        let encoded = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<ProjectId>(&encoded).unwrap(), id);
        assert!(serde_json::from_str::<ProjectId>("\"not-an-id\"").is_err());
        assert_eq!(id.to_string().parse::<ProjectId>().unwrap(), id);
    }
}
