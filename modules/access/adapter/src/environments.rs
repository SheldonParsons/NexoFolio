use nexofolio_access_contracts::{Environment, EnvironmentRef, validate_environment_name};
use nexofolio_common::{EnvironmentId, Error, Result};
use sqlx::{Postgres as Pg, Row, Transaction, postgres::PgRow};
use uuid::Uuid;
fn err(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "environments",
    }
}
pub(crate) fn from_row(row: &PgRow) -> Environment {
    Environment {
        id: row.get::<Uuid, _>("id").to_string().parse().expect("UUID"),
        name: row.get("name"),
    }
}

pub(crate) async fn resolve(
    tx: &mut Transaction<'_, Pg>,
    project: Uuid,
    reference: &EnvironmentRef,
) -> Result<Environment> {
    match reference {
        EnvironmentRef::ById(reference) => {
            let id: Uuid = reference.id.to_string().parse().expect("UUID");
            let row = sqlx::query("SELECT id,name FROM environments WHERE project_id=$1 AND id=$2")
                .bind(project)
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(err)?
                .ok_or(Error::NotFound)?;
            Ok(from_row(&row))
        }
        EnvironmentRef::ByName(reference) => {
            validate_environment_name(&reference.name)?;
            let existing=sqlx::query("SELECT e.id,e.name FROM environment_names n JOIN environments e ON e.id=n.environment_id WHERE n.project_id=$1 AND n.name=$2").bind(project).bind(&reference.name).fetch_optional(&mut **tx).await.map_err(err)?;
            if let Some(row) = existing {
                return Ok(from_row(&row));
            }
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(format!("environment-names:{project}"))
                .execute(&mut **tx)
                .await
                .map_err(err)?;
            let existing=sqlx::query("SELECT e.id,e.name FROM environment_names n JOIN environments e ON e.id=n.environment_id WHERE n.project_id=$1 AND n.name=$2").bind(project).bind(&reference.name).fetch_optional(&mut **tx).await.map_err(err)?;
            if let Some(row) = existing {
                return Ok(from_row(&row));
            }
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,$3)")
                .bind(id)
                .bind(project)
                .bind(&reference.name)
                .execute(&mut **tx)
                .await
                .map_err(err)?;
            sqlx::query(
                "INSERT INTO environment_names(project_id,name,environment_id) VALUES($1,$2,$3)",
            )
            .bind(project)
            .bind(&reference.name)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(err)?;
            Ok(Environment {
                id: id.to_string().parse().expect("UUID"),
                name: reference.name.clone(),
            })
        }
    }
}
pub(crate) async fn rename(
    tx: &mut Transaction<'_, Pg>,
    project: Uuid,
    id: EnvironmentId,
    name: &str,
) -> Result<Environment> {
    validate_environment_name(name)?;
    let uuid: Uuid = id.to_string().parse().expect("UUID");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("environment-names:{project}"))
        .execute(&mut **tx)
        .await
        .map_err(err)?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT environment_id FROM environment_names WHERE project_id=$1 AND name=$2",
    )
    .bind(project)
    .bind(name)
    .fetch_optional(&mut **tx)
    .await
    .map_err(err)?;
    if existing.is_some_and(|v| v != uuid) {
        return Err(Error::Conflict);
    }
    let row = sqlx::query(
        "UPDATE environments SET name=$1 WHERE id=$2 AND project_id=$3 RETURNING id,name",
    )
    .bind(name)
    .bind(uuid)
    .bind(project)
    .fetch_optional(&mut **tx)
    .await
    .map_err(err)?
    .ok_or(Error::NotFound)?;
    sqlx::query("INSERT INTO environment_names(project_id,name,environment_id) VALUES($1,$2,$3) ON CONFLICT(project_id,name) DO NOTHING").bind(project).bind(name).bind(uuid).execute(&mut **tx).await.map_err(err)?;
    Ok(from_row(&row))
}
