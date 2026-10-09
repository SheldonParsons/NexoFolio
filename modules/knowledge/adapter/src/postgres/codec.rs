//! Storage encodings shared within this adapter only.
use nexofolio_contracts::knowledge::Author;
use nexofolio_knowledge_contracts::{StoreError, StoreResult};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{fmt::Display, str::FromStr};
use uuid::Uuid;

pub(super) fn unavailable<E>(_: E) -> StoreError {
    StoreError::Unavailable
}

/// IDs are UUIDs underneath but do not expose them.
pub(super) fn uuid(id: impl Display) -> Uuid {
    Uuid::parse_str(&id.to_string()).expect("identifiers are UUIDs")
}

pub(super) fn id<T: FromStr>(uuid: Uuid) -> T {
    T::from_str(&uuid.to_string())
        .ok()
        .expect("identifiers are UUIDs")
}

/// Unit enums are stored as their serde name.
pub(super) fn text<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => unreachable!("stored enums serialise to strings"),
    }
}

pub(super) fn parse<T: DeserializeOwned>(name: String) -> StoreResult<T> {
    serde_json::from_value(Value::String(name)).map_err(unavailable)
}

/// Authors are stored whole: they carry which pass and which model wrote a value.
pub(super) fn author(value: Value) -> StoreResult<Author> {
    serde_json::from_value(value).map_err(unavailable)
}

pub(super) fn json<T: Serialize>(value: &T) -> StoreResult<Value> {
    serde_json::to_value(value).map_err(unavailable)
}
