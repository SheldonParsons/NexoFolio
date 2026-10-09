//! Links SQL, all executed on the caller's existing transaction.
use super::{
    codec::{author, id, json, parse, text, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::{EndpointId, LinkId, ProjectId};
use nexofolio_contracts::knowledge::Link;
use nexofolio_knowledge_contracts::LinkIdentity;
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};

macro_rules! link_columns {
    () => {
        "id, from_endpoint, from_field, to_endpoint, to_field, relation, evidence, author, at, context"
    };
}

fn link_row(row: &PgRow) -> StoreResult<Link> {
    Ok(Link {
        id: id(row.get("id")),
        from: id(row.get("from_endpoint")),
        from_field: row.get("from_field"),
        to: id(row.get("to_endpoint")),
        to_field: row.get("to_field"),
        relation: parse(row.get("relation"))?,
        evidence: row.get("evidence"),
        context: serde_json::from_value(row.get("context")).map_err(unavailable)?,
        author: author(row.get("author"))?,
        at: row.get("at"),
    })
}

impl PostgresTx {
    pub(super) async fn db_link(&mut self, id: LinkId) -> StoreResult<Option<Link>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE id = $1"
        ))
        .bind(uuid(id))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(link_row).transpose()
    }

    pub(super) async fn db_find_link(
        &mut self,
        identity: &LinkIdentity,
    ) -> StoreResult<Option<Link>> {
        // Matches the links_identity index: NULL fields compare as ''.
        let row = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE from_endpoint = $1 AND to_endpoint = $2",
            " AND relation = $3 AND coalesce(from_field, '') = $4",
            " AND coalesce(to_field, '') = $5 AND coalesce(context->>'environment','') = $6",
            " AND coalesce(context->'from_location', 'null'::jsonb) = $7",
            " AND coalesce(context->'to_location', 'null'::jsonb) = $8"
        ))
        .bind(uuid(identity.from))
        .bind(uuid(identity.to))
        .bind(text(identity.relation))
        .bind(identity.from_field.as_deref().unwrap_or(""))
        .bind(identity.to_field.as_deref().unwrap_or(""))
        .bind(
            identity
                .environment
                .map(|id| id.to_string())
                .unwrap_or_default(),
        )
        .bind(json(&identity.from_location)?)
        .bind(json(&identity.to_location)?)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(link_row).transpose()
    }

    pub(super) async fn db_links(&mut self, project: ProjectId) -> StoreResult<Vec<Link>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE project_id = $1 ORDER BY id"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(link_row).collect()
    }

    pub(super) async fn db_links_of(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Link>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            link_columns!(),
            " FROM links WHERE from_endpoint = $1 OR to_endpoint = $1 ORDER BY id"
        ))
        .bind(uuid(endpoint))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(link_row).collect()
    }

    pub(super) async fn db_put_link(&mut self, project: ProjectId, link: &Link) -> StoreResult<()> {
        // Conflict on identity, not on id: core mints an id before it knows
        // whether an equal link already exists.
        sqlx::query(concat!(
            "INSERT INTO links (project_id, ",
            link_columns!(),
            ") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            " ON CONFLICT (from_endpoint, to_endpoint, relation,",
            " coalesce(from_field, ''), coalesce(to_field, ''), coalesce(context->>'environment', ''),",
            " coalesce(context->'from_location', 'null'::jsonb), coalesce(context->'to_location', 'null'::jsonb))",
            " DO UPDATE SET evidence = $8, author = $9, at = $10, context = $11"
        ))
        .bind(uuid(project))
        .bind(uuid(link.id))
        .bind(uuid(link.from))
        .bind(link.from_field.as_deref())
        .bind(uuid(link.to))
        .bind(link.to_field.as_deref())
        .bind(text(link.relation))
        .bind(&link.evidence)
        .bind(json(&link.author)?)
        .bind(link.at)
        .bind(json(&link.context)?)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    pub(super) async fn db_remove_link(&mut self, id: LinkId) -> StoreResult<()> {
        sqlx::query("DELETE FROM links WHERE id = $1")
            .bind(uuid(id))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }
}
