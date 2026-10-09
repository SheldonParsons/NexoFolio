//! Rounds SQL, all executed on the caller's existing transaction.
use super::{
    codec::{author, id, json, parse, text, unavailable, uuid},
    transaction::PostgresTx,
};
use chrono::{DateTime, Utc};
use nexofolio_common::{ProjectId, RoundId};
use nexofolio_contracts::knowledge::{Round, RoundState};
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};

macro_rules! round_columns {
    () => {
        "id, project_id, author, started_at, finished_at, state, summary"
    };
}

fn round_row(row: &PgRow) -> StoreResult<Round> {
    Ok(Round {
        id: id(row.get("id")),
        project_id: id(row.get("project_id")),
        author: author(row.get("author"))?,
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        state: parse(row.get("state"))?,
        summary: row.get("summary"),
    })
}

impl PostgresTx {
    pub(super) async fn db_create_round(&mut self, round: &Round) -> StoreResult<()> {
        sqlx::query(concat!(
            "INSERT INTO rounds (",
            round_columns!(),
            ") VALUES ($1,$2,$3,$4,$5,$6,$7)"
        ))
        .bind(uuid(round.id))
        .bind(uuid(round.project_id))
        .bind(json(&round.author)?)
        .bind(round.started_at)
        .bind(round.finished_at)
        .bind(text(round.state))
        .bind(round.summary.as_deref())
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    pub(super) async fn db_round(&mut self, round: RoundId) -> StoreResult<Option<Round>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            round_columns!(),
            " FROM rounds WHERE id = $1"
        ))
        .bind(uuid(round))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(round_row).transpose()
    }

    pub(super) async fn db_finish_round(
        &mut self,
        round: RoundId,
        state: RoundState,
        summary: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        sqlx::query("UPDATE rounds SET state = $2, summary = $3, finished_at = $4 WHERE id = $1")
            .bind(uuid(round))
            .bind(text(state))
            .bind(summary)
            .bind(at)
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    pub(super) async fn db_rounds(
        &mut self,
        project: ProjectId,
        limit: usize,
    ) -> StoreResult<Vec<Round>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            round_columns!(),
            " FROM rounds WHERE project_id = $1 ORDER BY started_at DESC LIMIT $2"
        ))
        .bind(uuid(project))
        .bind(limit as i64)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(round_row).collect()
    }
}
