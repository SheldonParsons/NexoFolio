use super::*;
pub(super) async fn correlate_ui(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
    values: &[ValueOccurrence],
    start: i64,
    http_payload: &Value,
) -> Result<()> {
    let Some(interaction) = ctx.interaction_id else {
        return Ok(());
    };
    let rows = sqlx::query(
        r#"SELECT id,raw_hash,kind
            FROM capture_events
            WHERE project_id=$1
            AND environment_id=$2
            AND producer_id=$3
            AND context->>'page_instance_id'=$4
            AND context->>'frame_instance_id'=$5
            AND context->>'interaction_id'=$6
            AND kind='interaction'
            AND captured_at<=to_timestamp($7::double precision/1000)
            AND captured_at>=to_timestamp(($7-120000)::double precision/1000)
            AND raw_hash IS NOT NULL
            AND actor_id=$8
            AND context->>'browser_instance_id'=$9
            AND context->>'view_id'=$10
            ORDER BY captured_at DESC
            LIMIT 5"#,
    )
    .bind(uuid(event.project))
    .bind(uuid(event.env))
    .bind(event.producer)
    .bind(ctx.page_instance_id.to_string())
    .bind(ctx.frame_instance_id.to_string())
    .bind(interaction.to_string())
    .bind(start)
    .bind(event.actor)
    .bind(ctx.browser_instance_id.to_string())
    .bind(ctx.view_id.to_string())
    .fetch_all(&mut **tx)
    .await
    .map_err(err)?;
    // Interaction payloads are persisted as ui_interaction facts with actual labels and values.
    for row in rows {
        let data:Option<Value>=sqlx::query_scalar("SELECT f.data FROM evidence_facts f JOIN evidence_samples s ON s.fact_id=f.id WHERE s.event_id=$1 AND f.kind='ui_interaction' LIMIT 1").bind(row.get::<Uuid,_>("id")).fetch_optional(&mut **tx).await.map_err(err)?;
        if let Some(data) = data {
            match_ui_values(
                tx,
                event,
                ctx,
                &data,
                values,
                Some(row.get("id")),
                http_payload,
            )
            .await?;
        }
    }
    Ok(())
}
async fn match_ui_values(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
    data: &Value,
    values: &[ValueOccurrence],
    other: Option<Uuid>,
    http_payload: &Value,
) -> Result<()> {
    let target = &data["target"];
    let selected = &target["value"];
    let matching: Vec<_> = values
        .iter()
        .filter(|v| {
            v.direction == "request"
                && nexofolio_evidence::value_match(&selected["value"], &v.value).is_some()
        })
        .collect();
    if matching.len() != 1 || selected["state"] != "present" {
        record_omission(tx, event, ctx, data, values, other, http_payload).await?;
        return Ok(());
    }
    sqlx::query(r#"INSERT INTO evidence_ui_bindings(project_id,environment_id,actor_id,producer_id,browser_instance_id,page_instance_id,frame_instance_id,view_id,element_id,field_key,field_ref)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            ON CONFLICT DO NOTHING"#).bind(uuid(event.project)).bind(uuid(event.env)).bind(event.actor).bind(event.producer).bind(ctx.browser_instance_id).bind(ctx.page_instance_id).bind(ctx.frame_instance_id).bind(ctx.view_id).bind(target["element_id"].as_str().unwrap_or("")).bind(field_key(&matching[0].field)).bind(serde_json::to_value(&matching[0].field).unwrap()).execute(&mut **tx).await.map_err(err)?;
    let (http_event, ui_event) = if matches!(event.kind, CaptureKind::HttpExchange) {
        (event.id, other.ok_or(Error::Conflict)?)
    } else {
        (other.ok_or(Error::Conflict)?, event.id)
    };
    let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM evidence_ui_pairs WHERE http_event=$1 AND ui_event=$2 AND field_key=$3)").bind(http_event).bind(ui_event).bind(field_key(&matching[0].field)).fetch_one(&mut **tx).await.map_err(err)?;
    if duplicate {
        return Ok(());
    }
    let binding=save_fact(tx,event,&FactDraft{kind:"ui_field_label".into(),subject:serde_json::to_value(&matching[0].field).unwrap(),data:json!({"element_id":target["element_id"],"label":target["label"],"name":target["name"],"role":target["role"],"verification":"inferred"})},other).await?;
    sqlx::query("INSERT INTO evidence_ui_pairs(http_event,ui_event,field_key,fact_id) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(http_event).bind(ui_event).bind(field_key(&matching[0].field)).bind(binding).execute(&mut **tx).await.map_err(err)?;
    let labels: Vec<_> = target["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| {
            o["selected"] == true
                && o["value"]["state"] == "present"
                && o["value"]["value"] == selected["value"]
        })
        .filter_map(|o| o["label"].as_str())
        .collect();
    if labels.len() != 1 {
        return Ok(());
    }
    save_fact(tx,event,&FactDraft{kind:"enum_label_candidate".into(),subject:serde_json::to_value(&matching[0].field).unwrap(),data:json!({"state":"present","value":matching[0].value,"label":labels[0],"source":"ui_interaction","scope":{"page_url":ctx.page_url,"element_id":target["element_id"]},"complete_enum":false,"verification":"observed_mapping"})},other).await?;
    Ok(())
}
pub(super) async fn link_late_ui(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
    payload: &Value,
    blobs: &dyn nexofolio_evidence::BlobStore,
) -> Result<()> {
    if !matches!(event.kind, CaptureKind::Interaction) {
        return Ok(());
    }
    let Some(interaction) = ctx.interaction_id else {
        return Ok(());
    };
    let rows = sqlx::query(
        r#"SELECT id,raw_hash
            FROM capture_events
            WHERE project_id=$1
            AND environment_id=$2
            AND actor_id=$3
            AND producer_id=$4
            AND context->>'browser_instance_id'=$5
            AND context->>'page_instance_id'=$6
            AND context->>'frame_instance_id'=$7
            AND context->>'view_id'=$8
            AND context->>'interaction_id'=$9
            AND kind='http_exchange'
            AND raw_hash IS NOT NULL
            AND (context->>'request_started_at_ms')::bigint>=$10
            AND (context->>'request_started_at_ms')::bigint<=$10+120000
            ORDER BY received_at
            LIMIT 20"#,
    )
    .bind(uuid(event.project))
    .bind(uuid(event.env))
    .bind(event.actor)
    .bind(event.producer)
    .bind(ctx.browser_instance_id.to_string())
    .bind(ctx.page_instance_id.to_string())
    .bind(ctx.frame_instance_id.to_string())
    .bind(ctx.view_id.to_string())
    .bind(interaction.to_string())
    .bind(event.at.timestamp_millis())
    .fetch_all(&mut **tx)
    .await
    .map_err(err)?;
    for row in rows {
        let http_id: Uuid = row.get("id");
        let indexed=sqlx::query("SELECT field_ref,pointer,value FROM evidence_value_index WHERE event_id=$1 AND direction='request' LIMIT 512").bind(http_id).fetch_all(&mut **tx).await.map_err(err)?;
        let values = indexed
            .into_iter()
            .map(|r| {
                Ok(ValueOccurrence {
                    field: serde_json::from_value(r.get("field_ref"))
                        .map_err(|_| Error::Conflict)?,
                    pointer: r.get("pointer"),
                    value: r.get("value"),
                    direction: "request",
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let http_payload: Value = serde_json::from_slice(
            &blobs
                .get(event.project, &row.get::<String, _>("raw_hash"))
                .await?,
        )
        .map_err(|_| Error::Conflict)?;
        match_ui_values(
            tx,
            event,
            ctx,
            payload,
            &values,
            Some(http_id),
            &http_payload,
        )
        .await?;
    }
    Ok(())
}
pub(super) async fn record_omission(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
    ui: &Value,
    values: &[ValueOccurrence],
    other: Option<Uuid>,
    http: &Value,
) -> Result<()> {
    let target = &ui["target"];
    if target["value"]["state"] == "unknown" {
        return Ok(());
    }
    let fields=sqlx::query_scalar::<_,Value>("SELECT field_ref FROM evidence_ui_bindings WHERE project_id=$1 AND environment_id=$2 AND actor_id=$3 AND producer_id=$4 AND browser_instance_id=$5 AND page_instance_id=$6 AND frame_instance_id=$7 AND view_id=$8 AND element_id=$9 LIMIT 2")
 .bind(uuid(event.project)).bind(uuid(event.env)).bind(event.actor).bind(event.producer).bind(ctx.browser_instance_id).bind(ctx.page_instance_id).bind(ctx.frame_instance_id).bind(ctx.view_id).bind(target["element_id"].as_str().unwrap_or("")).fetch_all(&mut **tx).await.map_err(err)?;
    if fields.len() != 1 {
        return Ok(());
    }
    let field: FieldRef = serde_json::from_value(fields[0].clone()).map_err(|_| Error::Conflict)?;
    if values
        .iter()
        .any(|v| v.field.location == field.location && v.field.path == field.path)
    {
        return Ok(());
    }
    let absent = nexofolio_evidence::field_absent(http, &field);
    if !absent {
        return Ok(());
    }
    let (http_event, ui_event) = if matches!(event.kind, CaptureKind::HttpExchange) {
        (event.id, other.ok_or(Error::Conflict)?)
    } else {
        (other.ok_or(Error::Conflict)?, event.id)
    };
    let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM evidence_ui_pairs WHERE http_event=$1 AND ui_event=$2 AND field_key=$3)").bind(http_event).bind(ui_event).bind(field_key(&field)).fetch_one(&mut **tx).await.map_err(err)?;
    if duplicate {
        return Ok(());
    }
    let labels: Vec<_> = target["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["selected"] == true)
        .filter_map(|o| o["label"].as_str())
        .collect();
    let fact=save_fact(tx,event,&FactDraft{kind:"enum_label_candidate".into(),subject:serde_json::to_value(&field).unwrap(),data:json!({"state":"omitted","value":null,"label":if labels.len()==1{Some(labels[0])}else{None},"ui_value":target["value"],"source":"ui_interaction","scope":{"page_url":ctx.page_url,"element_id":target["element_id"]},"complete_enum":false,"verification":"inferred","behavior":"request_omits_field"})},other).await?;
    sqlx::query("INSERT INTO evidence_ui_pairs(http_event,ui_event,field_key,fact_id) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(http_event).bind(ui_event).bind(field_key(&field)).bind(fact).execute(&mut **tx).await.map_err(err)?;
    Ok(())
}

pub(super) async fn relation_counterexamples(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    event: &Event,
    ctx: &CaptureContext,
    value: &ValueOccurrence,
    start: i64,
) -> Result<()> {
    let Some(interaction) = ctx.interaction_id else {
        return Ok(());
    };
    let relations=sqlx::query("SELECT id,subject FROM evidence_facts WHERE project_id=$1 AND environment_id=$2 AND kind='parameter_link_candidate' AND subject->'target'=$3 LIMIT 64").bind(uuid(event.project)).bind(uuid(event.env)).bind(serde_json::to_value(&value.field).unwrap()).fetch_all(&mut **tx).await.map_err(err)?;
    for relation in relations {
        let subject: Value = relation.get("subject");
        let source: FieldRef =
            serde_json::from_value(subject["source"].clone()).map_err(|_| Error::Conflict)?;
        let rows = sqlx::query(
            r#"SELECT event_id,value
            FROM evidence_value_index
            WHERE project_id=$1
            AND environment_id=$2
            AND actor_id=$3
            AND producer_id=$4
            AND browser_instance_id=$5
            AND page_instance_id=$6
            AND frame_instance_id=$7
            AND view_id=$8
            AND interaction_id=$9
            AND field_key=$10
            AND direction='response'
            AND available_at<=$11
            AND available_at>=$11-120000
            AND event_id<>$12
            ORDER BY available_at DESC
            LIMIT 2"#,
        )
        .bind(uuid(event.project))
        .bind(uuid(event.env))
        .bind(event.actor)
        .bind(event.producer)
        .bind(ctx.browser_instance_id)
        .bind(ctx.page_instance_id)
        .bind(ctx.frame_instance_id)
        .bind(ctx.view_id)
        .bind(interaction)
        .bind(field_key(&source))
        .bind(start)
        .bind(event.id)
        .fetch_all(&mut **tx)
        .await
        .map_err(err)?;
        if rows.len() != 1 {
            continue;
        }
        let source_value: Value = rows[0].get("value");
        if !nexofolio_evidence::informative(&source_value)
            || nexofolio_evidence::value_match(&source_value, &value.value).is_some()
        {
            continue;
        }
        let relation_id: Uuid = relation.get("id");
        let fact = FactDraft {
            kind: "parameter_link_counterexample".into(),
            subject: json!({"relation_id":relation_id,"source":source,"target":value.field}),
            data: json!({"source_value":source_value,"target_value":value.value,"counterexample":true,"verification":"needs_review","reason":"Same recorded interaction, but values do not match; correlation is not causation."}),
        };
        let counter = save_fact(tx, event, &fact, Some(rows[0].get("event_id"))).await?;
        sqlx::query("UPDATE evidence_facts SET data=data||jsonb_build_object('conflict',true,'verification','needs_review') WHERE id=$1").bind(relation_id).execute(&mut **tx).await.map_err(err)?;
        sqlx::query("UPDATE evidence_sample_groups SET counterexample=true WHERE fact_id=$1")
            .bind(counter)
            .execute(&mut **tx)
            .await
            .map_err(err)?;
    }
    Ok(())
}
