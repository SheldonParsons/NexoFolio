use super::*;
pub(super) async fn link_values(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
    value: &ValueOccurrence,
    start: i64,
    end: i64,
) -> Result<()> {
    if value.direction == "request" {
        relation_counterexamples(tx, event, ctx, value, start).await?;
    }
    let other_direction = if value.direction == "response" {
        "request"
    } else {
        "response"
    };
    let rows = sqlx::query(
        r#"SELECT v.*
            FROM evidence_value_index v
            WHERE project_id=$1
            AND environment_id=$2
            AND producer_id=$3
            AND page_instance_id=$4
            AND frame_instance_id=$5
            AND (view_id=$6
            OR EXISTS(SELECT 1
            FROM capture_events ui
            WHERE ui.project_id=v.project_id
            AND ui.environment_id=v.environment_id
            AND ui.actor_id=v.actor_id
            AND ui.producer_id=v.producer_id
            AND ui.kind='interaction'
            AND ui.context->>'browser_instance_id'=v.browser_instance_id::text
            AND ui.context->>'page_instance_id'=v.page_instance_id::text
            AND ui.context->>'frame_instance_id'=v.frame_instance_id::text
            AND (($8='response'
            AND ui.context->>'view_id'=v.view_id::text
            AND ui.context->>'interaction_id'=$14::text
            AND ui.captured_at>=to_timestamp(v.available_at::double precision/1000)
            AND ui.captured_at<=to_timestamp($10::double precision/1000))
            OR ($8='request'
            AND ui.context->>'view_id'=$6::text
            AND ui.context->>'interaction_id'=v.interaction_id::text
            AND ui.captured_at>=to_timestamp($11::double precision/1000)
            AND ui.captured_at<=to_timestamp(v.started_at::double precision/1000)))))
            AND match_key=$7
            AND direction=$8
            AND event_id<>$9
            AND (($8='response'
            AND available_at<=$10
            AND available_at>=$10-120000)
            OR ($8='request'
            AND started_at>=$11
            AND started_at<=$11+120000))
            AND actor_id=$12
            AND browser_instance_id=$13
            ORDER BY available_at DESC
            LIMIT 20"#,
    )
    .bind(uuid(event.project))
    .bind(uuid(event.env))
    .bind(event.producer)
    .bind(ctx.page_instance_id)
    .bind(ctx.frame_instance_id)
    .bind(ctx.view_id)
    .bind(match_key(&value.value))
    .bind(other_direction)
    .bind(event.id)
    .bind(start)
    .bind(end)
    .bind(event.actor)
    .bind(ctx.browser_instance_id)
    .bind(ctx.interaction_id.map(|v| v.to_string()))
    .fetch_all(&mut **tx)
    .await
    .map_err(err)?;

    let search_complete = rows.len() < 20;
    for row in rows {
        let other: Value = row.get("value");
        let (source_value, target_value) = if value.direction == "response" {
            (&value.value, &other)
        } else {
            (&other, &value.value)
        };
        let Some(transform) = nexofolio_evidence::value_match(source_value, target_value) else {
            continue;
        };
        let counterpart: FieldRef =
            serde_json::from_value(row.get("field_ref")).map_err(|_| Error::Unavailable {
                component: "field_ref",
            })?;
        let (source, target, source_event, target_event) = if value.direction == "response" {
            (
                &value.field,
                &counterpart,
                event.id,
                row.get::<Uuid, _>("event_id"),
            )
        } else {
            (
                &counterpart,
                &value.field,
                row.get::<Uuid, _>("event_id"),
                event.id,
            )
        };
        if !nexofolio_evidence::eligible_relation(source, target) {
            continue;
        }
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM evidence_relation_pairs WHERE source_event=$1 AND target_event=$2 AND source_key=$3 AND target_key=$4)").bind(source_event).bind(target_event).bind(field_key(source)).bind(field_key(target)).fetch_one(&mut **tx).await.map_err(err)?;
        if exists {
            continue;
        }
        let count: i64 = sqlx::query_scalar(
            r#"SELECT count(DISTINCT field_key)
            FROM evidence_value_index
            WHERE project_id=$1
            AND environment_id=$2
            AND actor_id=$3
            AND producer_id=$4
            AND browser_instance_id=$5
            AND page_instance_id=$6
            AND frame_instance_id=$7
            AND view_id=$8
            AND match_key=$9
            AND direction='response'
            AND available_at<=$10
            AND available_at>=$10-120000"#,
        )
        .bind(uuid(event.project))
        .bind(uuid(event.env))
        .bind(event.actor)
        .bind(event.producer)
        .bind(ctx.browser_instance_id)
        .bind(ctx.page_instance_id)
        .bind(ctx.frame_instance_id)
        .bind(ctx.view_id)
        .bind(match_key(target_value))
        .bind(if value.direction == "response" {
            row.get::<i64, _>("started_at")
        } else {
            start
        })
        .fetch_one(&mut **tx)
        .await
        .map_err(err)?;
        let support = nexofolio_evidence::RelationSupport {
            alternative_sources: count,
            search_complete,
            cross_view_bridge: row.get::<Uuid, _>("view_id") != ctx.view_id,
            interaction_observed: ctx.interaction_id.is_some()
                && ctx.interaction_id == row.get::<Option<Uuid>, _>("interaction_id"),
        };
        let ambiguous = support.ambiguous();
        if ambiguous {
            sqlx::query("UPDATE evidence_facts SET data=data||'{\"ambiguous\":true}'::jsonb WHERE id IN (SELECT fact_id FROM evidence_relation_pairs WHERE target_event=$1 AND target_key=$2)").bind(target_event).bind(field_key(target)).execute(&mut **tx).await.map_err(err)?;
        }
        let fact = nexofolio_evidence::parameter_link_fact(source, target, transform, &support);
        let id = save_fact(tx, event, &fact, Some(row.get("event_id"))).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM evidence_relation_values WHERE fact_id=$1")
                .bind(id)
                .fetch_one(&mut **tx)
                .await
                .map_err(err)?;
        if count < 6 {
            sqlx::query("INSERT INTO evidence_relation_values(fact_id,value_hash) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(id).bind(hash(source_value)).execute(&mut **tx).await.map_err(err)?;
        }
        sqlx::query("UPDATE evidence_facts SET data=data||jsonb_build_object('distinct_values_lower_bound',(SELECT count(*) FROM evidence_relation_values WHERE fact_id=$1)) WHERE id=$1").bind(id).execute(&mut **tx).await.map_err(err)?;

        sqlx::query("INSERT INTO evidence_relation_pairs(source_event,target_event,source_key,target_key,fact_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(source_event).bind(target_event).bind(field_key(source)).bind(field_key(target)).bind(id).execute(&mut **tx).await.map_err(err)?;
        sqlx::query(
            r#"UPDATE evidence_facts
            SET data=data||'{"ambiguous":true}'::jsonb
            WHERE id IN (SELECT fact_id
            FROM evidence_relation_pairs
            WHERE target_event=$1
            AND target_key=$2)
            AND (SELECT count(DISTINCT source_key)
            FROM evidence_relation_pairs
            WHERE target_event=$1
            AND target_key=$2)>1"#,
        )
        .bind(target_event)
        .bind(field_key(target))
        .execute(&mut **tx)
        .await
        .map_err(err)?;
    }
    Ok(())
}
