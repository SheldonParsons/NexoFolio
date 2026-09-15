//! One project authorization rule for transactional knowledge writes and reads.
use nexofolio_contracts::{Error, ProjectId, Result, UserId};
use sqlx::{Row, Transaction};

pub(crate) async fn authorize_project(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    user: UserId,
    project: ProjectId,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT u.enabled, u.grants_synced,
         EXISTS(SELECT 1 FROM user_project_access a
                WHERE a.user_id=u.id AND a.project_id=p.id) AS allowed
         FROM users u JOIN projects p ON p.instance=u.instance
         WHERE u.id=$1 AND p.id=$2 FOR SHARE OF u",
    )
    .bind(user.to_string().parse::<uuid::Uuid>().expect("typed UUID"))
    .bind(
        project
            .to_string()
            .parse::<uuid::Uuid>()
            .expect("typed UUID"),
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| Error::Unavailable {
        component: "project_access",
    })?
    .ok_or(Error::NotFound)?;
    if !row.get::<bool, _>("enabled") {
        return Err(Error::Unauthenticated);
    }
    if !row.get::<bool, _>("grants_synced") {
        return Err(Error::Unavailable {
            component: "project_access",
        });
    }
    if !row.get::<bool, _>("allowed") {
        return Err(Error::Forbidden);
    }
    Ok(())
}
