use super::*;
pub(super) async fn save_fact(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    fact: &FactDraft,
    other: Option<Uuid>,
) -> Result<Uuid> {
    // Lock source events against expiry before adding representative references.
    let sources: Vec<_> = std::iter::once(event.id).chain(other).collect();
    sqlx::query("SELECT id FROM capture_events WHERE id=ANY($1) ORDER BY id FOR SHARE")
        .bind(&sources)
        .fetch_all(&mut **tx)
        .await
        .map_err(err)?;
    let key = if fact.kind == "parameter_link_candidate" {
        hash(&json!([fact.subject, fact.data["transform"]]))
    } else {
        hash(&json!([fact.subject, fact.data]))
    };
    let id:Uuid=sqlx::query_scalar(r#"INSERT INTO evidence_facts(id,project_id,environment_id,kind,subject,data,key_hash,first_seen,last_seen,subject_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$8,$9)
            ON CONFLICT(project_id,environment_id,kind,key_hash) DO UPDATE
            SET observations=evidence_facts.observations+1,first_seen=least(evidence_facts.first_seen,excluded.first_seen),last_seen=greatest(evidence_facts.last_seen,excluded.last_seen)
            RETURNING id"#)
  .bind(Uuid::new_v4()).bind(uuid(event.project)).bind(uuid(event.env)).bind(&fact.kind).bind(&fact.subject).bind(&fact.data).bind(key).bind(event.at).bind(hash(&fact.subject)).fetch_one(&mut **tx).await.map_err(err)?;
    if fact.kind == "parameter_link_candidate" {
        sqlx::query(r#"UPDATE evidence_facts
            SET data=data||jsonb_build_object('ambiguous',coalesce((data->>'ambiguous')::boolean,false)
            OR $2,'interaction_observed',coalesce((data->>'interaction_observed')::boolean,false)
            OR $3)
            WHERE id=$1"#).bind(id).bind(fact.data["ambiguous"].as_bool().unwrap_or(false)).bind(fact.data["interaction_observed"].as_bool().unwrap_or(false)).execute(&mut **tx).await.map_err(err)?;
    }
    if fact.kind == "enum_label_candidate" || fact.kind == "dictionary_mapping_candidate" {
        let conflicts: Vec<Uuid> = sqlx::query_scalar(
            r#"SELECT id
            FROM evidence_facts
            WHERE project_id=$1
            AND environment_id=$2
            AND kind=$3
            AND subject_hash=$4
            AND id<>$5
            AND data->'state'=$6
            AND data->'value'=$7
            AND data->'label' IS DISTINCT
            FROM $8
            AND data->'scope' IS NOT DISTINCT
            FROM $9"#,
        )
        .bind(uuid(event.project))
        .bind(uuid(event.env))
        .bind(&fact.kind)
        .bind(hash(&fact.subject))
        .bind(id)
        .bind(&fact.data["state"])
        .bind(&fact.data["value"])
        .bind(&fact.data["label"])
        .bind(fact.data.get("scope"))
        .fetch_all(&mut **tx)
        .await
        .map_err(err)?;
        if !conflicts.is_empty() {
            let mut all = conflicts;
            all.push(id);
            sqlx::query("UPDATE evidence_facts SET data=data||'{\"conflict\":true,\"verification\":\"needs_review\"}'::jsonb WHERE id=ANY($1)").bind(&all).execute(&mut **tx).await.map_err(err)?;
            sqlx::query(
                "UPDATE evidence_sample_groups SET counterexample=true WHERE fact_id=ANY($1)",
            )
            .bind(all)
            .execute(&mut **tx)
            .await
            .map_err(err)?;
        }
    }
    let mut events: Vec<_> = std::iter::once(event.id).chain(other).collect();
    events.sort();
    events.dedup();
    let support_key = hash(&json!(events));
    sqlx::query(r#"INSERT INTO evidence_sample_groups(fact_id,support_key,event_ids,observed_at,first_sample,counterexample)
            VALUES($1,$2,$3,$4,NOT EXISTS(SELECT 1
            FROM evidence_sample_groups
            WHERE fact_id=$1),coalesce((SELECT (data->>'conflict')::boolean
            FROM evidence_facts
            WHERE id=$1),false))
            ON CONFLICT DO NOTHING"#).bind(id).bind(support_key).bind(&events).bind(event.at).execute(&mut **tx).await.map_err(err)?;
    sqlx::query(r#"DELETE
            FROM evidence_sample_groups
            WHERE fact_id=$1
            AND support_key NOT IN (SELECT support_key
            FROM evidence_sample_groups
            WHERE fact_id=$1
            ORDER BY CASE WHEN first_sample THEN 0 WHEN observed_at=(SELECT max(observed_at)
            FROM evidence_sample_groups
            WHERE fact_id=$1) THEN 1 WHEN counterexample THEN 2 ELSE 3 END,observed_at DESC,support_key
            LIMIT 5)"#).bind(id).execute(&mut **tx).await.map_err(err)?;
    // Pins preserve candidate/release originals separately; this table holds only working samples.
    sqlx::query("DELETE FROM evidence_samples WHERE fact_id=$1 AND event_id NOT IN(SELECT unnest(event_ids) FROM evidence_sample_groups WHERE fact_id=$1)").bind(id).execute(&mut **tx).await.map_err(err)?;
    sqlx::query(r#"INSERT INTO evidence_samples(fact_id,event_id,first_sample,counterexample) SELECT fact_id,unnest(event_ids),first_sample,counterexample
            FROM evidence_sample_groups
            WHERE fact_id=$1
            ON CONFLICT DO NOTHING"#).bind(id).execute(&mut **tx).await.map_err(err)?;
    Ok(id)
}
