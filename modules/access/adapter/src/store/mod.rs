//! `PostgresAccess` implements every access contract over the `access` schema,
//! one contract per file. `scope` implements the shared collect contracts.
mod environments;
mod login;
mod projects;
mod scope;
mod sessions;

use crate::{Postgres, session_crypto::SessionCrypto};
use nexofolio_access_contracts::UserProfile;
use nexofolio_common::{Error, Result, Secret};
use sqlx::{Row, postgres::PgRow};
use std::{fmt::Display, str::FromStr, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresAccess {
    database: Postgres,
    crypto: Arc<SessionCrypto>,
}

impl PostgresAccess {
    pub fn new(database: Postgres, key: &Secret) -> Result<Self> {
        Ok(Self {
            database,
            crypto: Arc::new(SessionCrypto::new(key)?),
        })
    }
}

fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "access_database",
    }
}

/// Typed IDs to database UUIDs and back.
fn uuid(id: impl Display) -> Uuid {
    id.to_string().parse().expect("typed UUID")
}
fn typed<T: FromStr>(id: Uuid) -> T {
    id.to_string()
        .parse()
        .unwrap_or_else(|_| unreachable!("database UUID"))
}

fn profile(row: &PgRow) -> Result<UserProfile> {
    Ok(UserProfile {
        id: typed(row.try_get("id").map_err(db_error)?),
        account: row.try_get("account").map_err(db_error)?,
        display_name: row.try_get("display_name").map_err(db_error)?,
        enabled: row.try_get("enabled").map_err(db_error)?,
    })
}
