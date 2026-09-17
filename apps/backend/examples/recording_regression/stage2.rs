//! Stage 2 diagnostics on an explicitly isolated restored recording; never infer causation.
use nexofolio_contracts::*;
use nexofolio_evidence::{dictionary_facts, extract_http, informative, relation_field};
use nexofolio_infrastructure::{FileBlobStore, Postgres, PostgresCaptureStore};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub fn run(input: &Value) -> Value {
    let url = input["database_url"].as_str().unwrap();
    assert!(input["isolated_database"] == true);
    assert!(
        url.contains("/stage2_audit"),
        "only the isolated audit database is permitted"
    );
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db=Postgres::new(&Secret::new(url), 5, Duration::from_secs(2)).unwrap();
        db.migrate().await.unwrap();
        let sql=sqlx::PgPool::connect(url).await.unwrap();
        let blobs=Arc::new(FileBlobStore::new(input["blob_root"].as_str().unwrap().into()));
        let store=PostgresCaptureStore::new(db.clone(),blobs);
        let mut processed=0;
        while store.process_evidence_one().await.unwrap() {
            processed+=1;
            assert!(processed<=500, "bounded fixed-dataset replay");
        }
        let catalog:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('interface_id',d.id,'method',d.method,'path',d.path,'environments',jsonb_agg(jsonb_build_object('environment_id',e.id,'environment_name',e.name,'revision_id',r.id,'definition',r.definition) ORDER BY e.id)) FROM interface_documents d JOIN interface_environment_current c ON c.interface_id=d.id JOIN interface_observed_revisions r ON r.id=c.current_revision_id JOIN environments e ON e.id=c.environment_id GROUP BY d.id").fetch_all(&sql).await.unwrap();
        let interfaces:Vec<CatalogInterface>=catalog.into_iter().map(|v|serde_json::from_value(v).unwrap()).collect();
        let fields=nexofolio_rebuild::snapshot_fields(&interfaces,&[]);
        let known:std::collections::HashSet<_>=fields.iter().map(|f|serde_json::to_value(&f.reference).unwrap().to_string()).collect();
        let mut outside=std::collections::HashSet::new();
        let mut extraction=BTreeMap::<String,usize>::new();
        for case in input["http"].as_array().unwrap() {
            let field:FieldRef=serde_json::from_value(case["field"].clone()).unwrap();
            let path=case["path_identity"].as_object().map(|_|serde_json::from_value(case["path_identity"].clone()).unwrap());
            let output=extract_http(&case["payload"],field,path.as_ref());
            *extraction.entry("http".into()).or_default()+=1;
            *extraction.entry("scalar_values".into()).or_default()+=output.values.len();
            for value in &output.values {
                let reference=serde_json::to_value(&value.field).unwrap().to_string();
                if !known.contains(&reference) { outside.insert(reference); }
            }
            *extraction.entry("dictionary_mappings".into()).or_default()+=dictionary_facts(&case["payload"],&output.values).len();
            for limitation in output.limitations { *extraction.entry(limitation).or_default()+=1; }
        }
        let facts:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('kind',kind,'count',count(*),'observations',sum(observations)) FROM evidence_facts GROUP BY kind ORDER BY kind").fetch_all(&sql).await.unwrap();
        let relations:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',f.id,'source_ref',f.subject->'source','target_ref',f.subject->'target','source_api',s.path,'source_field',f.subject->'source'->>'path','target_api',t.path,'target_field',f.subject->'target'->>'path','data',f.data,'observations',f.observations) FROM evidence_facts f JOIN interface_documents s ON s.id::text=f.subject->'source'->>'interface_id' JOIN interface_documents t ON t.id::text=f.subject->'target'->>'interface_id' WHERE f.kind='parameter_link_candidate' ORDER BY s.path,t.path,f.id").fetch_all(&sql).await.unwrap();
        let statuses:Value=sqlx::query_scalar("SELECT jsonb_object_agg(evidence_status,n) FROM (SELECT evidence_status,count(*) n FROM capture_events GROUP BY evidence_status) x").fetch_one(&sql).await.unwrap();
        let provenance:Value=sqlx::query_scalar("SELECT jsonb_build_object('unadopted_value_facts',(SELECT count(*) FROM evidence_facts WHERE kind='observed_value' AND subject ? 'observation'),'unadopted_claiming_revision',(SELECT count(*) FROM evidence_facts WHERE subject ? 'observation' AND subject ? 'revision_id'),'coverage_rows',(SELECT count(*) FROM capture_events WHERE evidence_coverage IS NOT NULL),'legacy_facts',(SELECT count(*) FROM evidence_facts WHERE data->>'evidence_rule_version'='legacy'))").fetch_one(&sql).await.unwrap();
        let summary=json!({"provenance":provenance,"current_snapshot_fields":fields.len(),"observed_fields_outside_current_definition":outside.len(),"processed":processed,"statuses":statuses,"extraction":extraction,"facts":facts,"relations":relations,"pure_probes":pure_probes(),"model_calls":0});
        sql.close().await;db.close().await;summary
    })
}
fn pure_probes() -> Value {
    let field = FieldRef {
        interface_id: InterfaceId::new(),
        environment_id: EnvironmentId::new(),
        revision_id: RevisionId::new(),
        location: "response.body".into(),
        path: "/sort".into(),
    };
    let payload = |url: &str, body: Value| json!({"request":{"url":url,"body":{"state":"none"}},"response":{"body":{"state":"complete","encoding":"text","content":body.to_string()}}});
    let mut body = json!({"a":vec![json!(23);520]});
    body["z_record_id"] = json!(987654);
    let limited = extract_http(
        &payload("https://example.test/orders", body),
        field.clone(),
        None,
    );
    let mut request = payload(
        "https://example.test/dict/status",
        json!({"items":[{"value":2,"label":"开放"}]}),
    );
    request["request"]["body"] =
        json!({"state":"complete","encoding":"text","content":"{\"region\":\"north\"}"});
    let extracted = extract_http(&request, field.clone(), None);
    let mapping = dictionary_facts(&request, &extracted.values);
    let mut query_only = request.clone();
    query_only["request"]["url"] = json!("https://example.test/search?return=dict");
    let mut credential = field.clone();
    credential.path = "/tokenValue".into();
    json!({
        "sort_23_is_currently_relation_eligible":informative(&json!(23))&&relation_field(&field.clone().into()),
        "token_value_field_is_currently_relation_eligible":relation_field(&credential.into()),
        "budget_limitation_emitted":limited.limitations.contains(&"EVIDENCE_EXTRACTION_LIMIT".to_string()),
        "budget_tail_field_missing":!limited.values.iter().any(|v|v.field.path=="/z_record_id"),
        "dictionary_mapping_has_request_body_scope":mapping[0].data.get("request_body").is_some()||mapping[0].data.get("scope").is_some(),
        "query_keyword_alone_triggers_dictionary_mapping":!dictionary_facts(&query_only,&extracted.values).is_empty()
    })
}
