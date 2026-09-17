use super::*;

/// One bounded join after the event's whole index is written. Source and target arrival
/// use exactly the same temporal/context predicate; matching never reads a partial event index.
pub(super) async fn link_event(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
) -> Result<()> {
    let rows=sqlx::query(r#"SELECT DISTINCT ON (s.event_id,t.event_id,s.field_key,t.field_key)
        s.event_id AS source_event,t.event_id AS target_event,s.field_key AS source_key,t.field_key AS target_key,
        s.field_ref AS source,t.field_ref AS target,s.value AS source_value,t.value AS target_value,
        s.view_id<>t.view_id AS bridge,
        coalesce(s.interaction_id=t.interaction_id,false) AS interaction,
        coalesce((se.evidence_coverage->>'response_complete')::boolean,false)
        AND coalesce((te.evidence_coverage->>'request_complete')::boolean,false) AS complete
        FROM evidence_value_index s JOIN evidence_value_index t
        ON s.project_id=t.project_id AND s.environment_id=t.environment_id
        AND s.actor_id=t.actor_id AND s.producer_id=t.producer_id
        AND s.browser_instance_id=t.browser_instance_id AND s.page_instance_id=t.page_instance_id
        AND s.frame_instance_id=t.frame_instance_id AND s.match_key=t.match_key
        AND s.direction='response' AND t.direction='request' AND s.event_id<>t.event_id
        AND s.available_at<=t.started_at AND s.available_at>=t.started_at-120000
        JOIN capture_events se ON se.id=s.event_id JOIN capture_events te ON te.id=t.event_id
        WHERE s.project_id=$1 AND (s.event_id=$2 OR t.event_id=$2) AND s.match_key<>''
        AND se.evidence_coverage IS NOT NULL AND te.evidence_coverage IS NOT NULL
        AND (s.view_id=t.view_id OR EXISTS(SELECT 1 FROM capture_events ui
          WHERE ui.kind='interaction' AND ui.project_id=s.project_id AND ui.environment_id=s.environment_id
          AND ui.actor_id=s.actor_id AND ui.producer_id=s.producer_id
          AND ui.context->>'browser_instance_id'=s.browser_instance_id::text
          AND ui.context->>'page_instance_id'=s.page_instance_id::text
          AND ui.context->>'frame_instance_id'=s.frame_instance_id::text
          AND ui.context->>'view_id'=s.view_id::text
          AND ui.context->>'interaction_id'=t.interaction_id::text
          AND ui.captured_at>=to_timestamp(s.available_at::double precision/1000)
          AND ui.captured_at<=to_timestamp(t.started_at::double precision/1000)))
        ORDER BY s.event_id,t.event_id,s.field_key,t.field_key,s.value,t.value,s.pointer,t.pointer
        LIMIT 513"#).bind(uuid(event.project)).bind(event.id).fetch_all(&mut **tx).await.map_err(err)?;
    // Roll back this event rather than committing an arbitrary prefix as a complete search.
    if rows.len() > 512 {
        return Err(Error::Unavailable {
            component: "evidence_relation_budget",
        });
    }
    for row in rows {
        let source: EvidenceFieldRef =
            serde_json::from_value(row.get("source")).map_err(|_| Error::Conflict)?;
        let target: EvidenceFieldRef =
            serde_json::from_value(row.get("target")).map_err(|_| Error::Conflict)?;
        if !nexofolio_evidence::eligible_relation(&source, &target) {
            continue;
        }
        let source_value: Value = row.get("source_value");
        let target_value: Value = row.get("target_value");
        let Some(transform) = nexofolio_evidence::value_match(&source_value, &target_value) else {
            continue;
        };
        let source_event: Uuid = row.get("source_event");
        let target_event: Uuid = row.get("target_event");
        let source_key: String = row.get("source_key");
        let target_key: String = row.get("target_key");
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM evidence_relation_pairs WHERE source_event=$1 AND target_event=$2 AND source_key=$3 AND target_key=$4)")
            .bind(source_event).bind(target_event).bind(&source_key).bind(&target_key).fetch_one(&mut **tx).await.map_err(err)?;
        if exists {
            continue;
        }
        let support = nexofolio_evidence::RelationSupport {
            alternative_sources: 1,
            search_complete: row.get("complete"),
            cross_view_bridge: row.get("bridge"),
            interaction_observed: row.get("interaction"),
        };
        let fact = nexofolio_evidence::parameter_link_fact(&source, &target, transform, &support);
        let other = if event.id == source_event {
            target_event
        } else {
            source_event
        };
        let id = save_fact(tx, event, &fact, Some(other)).await?;
        sqlx::query("INSERT INTO evidence_relation_values(fact_id,value_hash) SELECT $1,$2 WHERE (SELECT count(*) FROM evidence_relation_values WHERE fact_id=$1)<6 ON CONFLICT DO NOTHING")
            .bind(id).bind(hash(&source_value)).execute(&mut **tx).await.map_err(err)?;
        sqlx::query("UPDATE evidence_facts SET data=data||jsonb_build_object('distinct_values_lower_bound',(SELECT count(*) FROM evidence_relation_values WHERE fact_id=$1)) WHERE id=$1")
            .bind(id).execute(&mut **tx).await.map_err(err)?;
        sqlx::query("INSERT INTO evidence_relation_pairs(source_event,target_event,source_key,target_key,fact_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
            .bind(source_event).bind(target_event).bind(&source_key).bind(&target_key).bind(id).execute(&mut **tx).await.map_err(err)?;
        sqlx::query(r#"UPDATE evidence_facts SET data=data||'{"ambiguous":true}'::jsonb
            WHERE id IN (SELECT p.fact_id FROM evidence_relation_pairs p JOIN evidence_facts f ON f.id=p.fact_id WHERE p.target_event=$1 AND p.target_key=$2 AND f.kind='parameter_link_candidate')
            AND (SELECT count(DISTINCT p.source_key) FROM evidence_relation_pairs p JOIN evidence_facts f ON f.id=p.fact_id WHERE p.target_event=$1 AND p.target_key=$2 AND f.kind='parameter_link_candidate')>1"#)
            .bind(target_event).bind(&target_key).execute(&mut **tx).await.map_err(err)?;
    }
    recheck_counterexamples(tx, event).await?;
    Ok(())
}

// A newly supported relation can explain an earlier indexed request. Revisit only
// matching field references in this page/context, so late arrival cannot hide a counterexample.
async fn recheck_counterexamples(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
) -> Result<()> {
    let Some(ctx) = &event.context else {
        return Ok(());
    };
    let rows = sqlx::query(
        r#"SELECT e.*,v.field_ref,v.pointer,v.value,v.started_at
        FROM evidence_value_index v JOIN capture_events e ON e.id=v.event_id
        WHERE v.project_id=$1 AND v.environment_id=$3 AND v.actor_id=$4 AND v.producer_id=$5
        AND v.browser_instance_id=$6 AND v.page_instance_id=$7 AND v.frame_instance_id=$8
        AND v.direction='request' AND v.match_key<>'' AND e.raw_hash IS NOT NULL AND e.evidence_coverage IS NOT NULL
        AND EXISTS(SELECT 1 FROM evidence_facts f WHERE f.project_id=v.project_id
          AND f.kind='parameter_link_candidate' AND f.data->>'evidence_rule_version'='evidence-2'
          AND f.subject->'target'=v.field_ref
          AND (v.event_id=$2 OR EXISTS(SELECT 1 FROM evidence_relation_pairs p
             WHERE p.fact_id=f.id AND (p.source_event=$2 OR p.target_event=$2))))
        ORDER BY v.event_id,v.field_key,v.pointer LIMIT 513"#,
    )
    .bind(uuid(event.project))
    .bind(event.id)
    .bind(uuid(event.env))
    .bind(event.actor)
    .bind(event.producer)
    .bind(ctx.browser_instance_id)
    .bind(ctx.page_instance_id)
    .bind(ctx.frame_instance_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(err)?;
    if rows.len() > 512 {
        return Err(Error::Unavailable {
            component: "evidence_relation_budget",
        });
    }
    for row in rows {
        let target = Event::from_row(&row)?;
        let value = ValueOccurrence {
            field: serde_json::from_value(row.get("field_ref")).map_err(|_| Error::Conflict)?,
            pointer: row.get("pointer"),
            value: row.get("value"),
            direction: "request",
        };
        relation_counterexamples(
            tx,
            &target,
            target.context.as_ref().ok_or(Error::Conflict)?,
            &value,
            row.get("started_at"),
        )
        .await?;
    }
    Ok(())
}

/// A delayed interaction can supply the cross-view bridge after both HTTP events
/// have already been indexed. Revisit only its exact operation in the same owner/context.
pub(super) async fn late_interaction(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
) -> Result<()> {
    let Some(action) = ctx.interaction_id else {
        return Ok(());
    };
    let rows = sqlx::query(
        r#"SELECT * FROM capture_events WHERE project_id=$1 AND environment_id=$2
        AND actor_id=$3 AND producer_id=$4 AND kind='http_exchange' AND evidence_status='completed'
        AND context->>'browser_instance_id'=$5 AND context->>'page_instance_id'=$6
        AND context->>'frame_instance_id'=$7 AND context->>'interaction_id'=$8
        AND (context->>'request_started_at_ms')::bigint BETWEEN $9 AND $9+120000
        ORDER BY captured_at,id LIMIT 65"#,
    )
    .bind(uuid(event.project))
    .bind(uuid(event.env))
    .bind(event.actor)
    .bind(event.producer)
    .bind(ctx.browser_instance_id.to_string())
    .bind(ctx.page_instance_id.to_string())
    .bind(ctx.frame_instance_id.to_string())
    .bind(action.to_string())
    .bind(event.at.timestamp_millis())
    .fetch_all(&mut **tx)
    .await
    .map_err(err)?;
    if rows.len() > 64 {
        return Err(Error::Unavailable {
            component: "evidence_relation_budget",
        });
    }
    for row in rows {
        link_event(tx, &Event::from_row(&row)?).await?;
    }
    Ok(())
}
