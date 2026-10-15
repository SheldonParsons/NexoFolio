//! Knowledge-owned SQL only. Endpoint metadata arrives through the authorized read port.
use super::{
    codec::{json, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_contracts::knowledge::{KnowledgeSearch, KnowledgeSearchPage};
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::Row;

impl PostgresTx {
    pub(super) async fn db_search(
        &mut self,
        q: &KnowledgeSearch,
    ) -> StoreResult<KnowledgeSearchPage> {
        // Stored text cannot contain NUL; preserve the literal no-match result
        // without passing an unsupported PostgreSQL text parameter.
        if q.words.iter().any(|w| w.contains('\0')) {
            return Ok(q.select(&[], &[]));
        }
        // ICU root casing matches Unicode lowercasing, unlike C-locale lower().
        // Preserve the reference behavior on legacy SQL_ASCII/non-ICU databases.
        let unicode: bool = sqlx::query_scalar("SELECT current_setting('server_encoding')='UTF8' AND EXISTS (SELECT 1 FROM pg_collation WHERE collname='und-x-icu')")
            .fetch_one(&mut *self.tx).await.map_err(unavailable)?;
        if !unicode {
            let notes = self.db_endpoint_notes(q.project).await?;
            let fields = if q.fields {
                self.db_project_field_notes(q.project).await?
            } else {
                Vec::new()
            };
            return Ok(q.select(&notes, &fields));
        }
        let row = sqlx::query(r#"
WITH scope AS (
  SELECT v, ord FROM jsonb_array_elements($2::jsonb) WITH ORDINALITY a(v,ord)
), matched AS MATERIALIZED (
  SELECT s.ord, f.path,
    jsonb_build_object('endpoint',s.v->'id','name',n.name,'purpose',left(n.purpose,320),
      'field',CASE WHEN $3 THEN jsonb_build_object('key',f.path,'name',f.context->'display_name',
        'location',f.context->'location','description',left(f.text,320)) ELSE NULL END) AS hit
  FROM scope s
  LEFT JOIN endpoint_notes n ON n.project_id=$1 AND n.endpoint_id=(s.v->>'id')::uuid
  LEFT JOIN field_notes f ON $3 AND f.project_id=$1 AND f.endpoint_id=(s.v->>'id')::uuid
  WHERE (NOT $3 OR (f.path IS NOT NULL
    AND NOT (s.v->'excluded_keys' ? f.path)
    AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements_text(s.v->'excluded_prefixes') p
                    WHERE starts_with(f.path,p))))
  AND NOT EXISTS (SELECT 1 FROM unnest($4::text[]) w WHERE strpos(lower(
    CASE WHEN $3 THEN f.path||' '||coalesce(f.context->>'display_name','')||' '||f.text
    ELSE (s.v->>'method')||' '||(s.v->>'path')||' '||coalesce(n.name,'')||' '||coalesce(n.purpose,'') END
    COLLATE "und-x-icu"), w)=0)
), selected AS (
  SELECT hit FROM matched ORDER BY ord,path COLLATE "C" OFFSET $5 LIMIT $6
)
SELECT (SELECT count(*) FROM matched) AS total,
       (SELECT md5(coalesce(string_agg(md5(hit::text),'' ORDER BY ord,path COLLATE "C"),'')) FROM matched) AS revision,
       coalesce((SELECT jsonb_agg(hit) FROM selected),'[]'::jsonb) AS items
"#)
            .bind(uuid(q.project)).bind(json(&q.endpoints)?).bind(q.fields).bind(&q.words)
            .bind(i64::try_from(q.offset).unwrap_or(i64::MAX)).bind(q.limit as i64)
            .fetch_one(&mut *self.tx).await.map_err(unavailable)?;
        Ok(KnowledgeSearchPage {
            total: row.get::<i64, _>("total") as usize,
            revision: row.get("revision"),
            items: serde_json::from_value(row.get("items")).map_err(unavailable)?,
        })
    }
}
