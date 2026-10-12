use super::support::*;
use nexofolio_access_contracts::{Environments, SessionPrincipal};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{
    knowledge::{KnowledgeCandidates, KnowledgeSnapshot},
    project_lifecycle::ProjectLifecycle,
};
use serde_json::{Value, json};

async fn knowledge(
    h: &Harness,
    project: ProjectId,
    endpoints: &[uuid::Uuid],
    user: &SessionPrincipal,
) -> uuid::Uuid {
    let p = uuid(project);
    let folder = uuid::Uuid::new_v4();
    let round = uuid::Uuid::new_v4();
    let author = json!({"by":"person","user":user.user_id});
    sqlx::query("INSERT INTO knowledge.rounds(id,project_id,author,state,finished_at) VALUES($1,$2,$3,'done',now())").bind(round).bind(p).bind(&author).execute(&h.sql).await.unwrap();
    sqlx::query("INSERT INTO knowledge.revisions(round_id,seq,target,before) VALUES($1,1,'{}','{\"retained\":\"synthetic old content\"}')").bind(round).execute(&h.sql).await.unwrap();
    sqlx::query(
        "INSERT INTO knowledge.folders(id,project_id,name,author) VALUES($1,$2,'测试目录',$3)",
    )
    .bind(folder)
    .bind(p)
    .bind(&author)
    .execute(&h.sql)
    .await
    .unwrap();
    sqlx::query("INSERT INTO knowledge.placements(endpoint_id,project_id,folder_id,author) VALUES($1,$2,$3,$4)").bind(endpoints[0]).bind(p).bind(folder).bind(&author).execute(&h.sql).await.unwrap();
    sqlx::query("INSERT INTO knowledge.endpoint_notes(endpoint_id,project_id,name,purpose,author) VALUES($1,$2,'测试接口','synthetic private reset fixture',$3)").bind(endpoints[0]).bind(p).bind(&author).execute(&h.sql).await.unwrap();
    sqlx::query("INSERT INTO knowledge.field_notes(endpoint_id,path,project_id,text,author) VALUES($1,'request_body:id',$2,'synthetic meaning',$3)").bind(endpoints[0]).bind(p).bind(&author).execute(&h.sql).await.unwrap();
    sqlx::query("INSERT INTO knowledge.links(id,project_id,from_endpoint,to_endpoint,relation,evidence,author) VALUES($1,$2,$3,$4,'dictionary','synthetic relation evidence',$5)").bind(uuid::Uuid::new_v4()).bind(p).bind(endpoints[0]).bind(endpoints[1]).bind(&author).execute(&h.sql).await.unwrap();
    let snapshot = serde_json::to_value(KnowledgeSnapshot::default()).unwrap();
    let mut last = uuid::Uuid::nil();
    for state in ["ready", "partial", "applied", "discarded", "interrupted"] {
        let id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds,parent) VALUES($1,$2,$3,now(),now(),'fixture',$4,$4,'[]','{}','[]',$5)").bind(id).bind(p).bind(state).bind(&snapshot).bind(if last.is_nil(){None}else{Some(last)}).execute(&h.sql).await.unwrap();
        last = id;
    }
    sqlx::query("INSERT INTO knowledge.field_value_archives(candidate_id,project_id,archive) VALUES($1,$2,'synthetic archived observations')").bind(last).bind(p).execute(&h.sql).await.unwrap();
    last
}
async fn setup() -> (
    Harness,
    User,
    ProjectId,
    nexofolio_access_contracts::Environment,
    Value,
    Vec<uuid::Uuid>,
    uuid::Uuid,
) {
    let h = Harness::new().await;
    let admin = h.user("A80646").await;
    let p = h.project("云售后").await;
    let env = h
        .store
        .create_environment(&admin.principal, p, "测试环境")
        .await
        .unwrap();
    let (batch, ids) = h.populate(p, env.id).await;
    let candidate = knowledge(&h, p, &ids, &admin.principal).await;
    (h, admin, p, env, batch, ids, candidate)
}
async fn count(h: &Harness, table: &str, p: ProjectId) -> i64 {
    assert!(
        [
            "knowledge.folders",
            "knowledge.placements",
            "knowledge.endpoint_notes",
            "knowledge.field_notes",
            "knowledge.links",
            "knowledge.rounds",
            "knowledge.candidates",
            "knowledge.field_value_archives"
        ]
        .contains(&table)
    );
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE project_id=$1"
    )))
    .bind(uuid(p))
    .fetch_one(&h.sql)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn clear_removes_every_content_layer_preserves_configuration_and_fences_old_data() {
    let (h, admin, p, env, batch, ids, candidate) = setup().await;
    let other = h.project("其他项目").await;
    let other_env = h
        .store
        .create_environment(&admin.principal, other, "测试环境")
        .await
        .unwrap();
    h.populate(other, other_env.id).await;
    // An independent project using this project's grants must survive the reset.
    let child = h
        .store
        .create_local_project(&admin.principal, "巴西售后", &[p])
        .await
        .unwrap();
    let child_env = h
        .store
        .create_environment(&admin.principal, child.project_id, "测试环境")
        .await
        .unwrap();
    h.populate(child.project_id, child_env.id).await;
    h.store
        .rename_environment(&admin.principal, p, env.id, "灰度环境")
        .await
        .unwrap();
    sqlx::query("UPDATE observe.service_addresses SET manual_verdict='own' WHERE project_id=$1")
        .bind(uuid(p))
        .execute(&h.sql)
        .await
        .unwrap();
    let config_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM access.environment_sites WHERE environment_id=$1")
            .bind(uuid(env.id))
            .fetch_one(&h.sql)
            .await
            .unwrap();
    let path = format!("/v1/project-management/projects/{p}");
    let (status, preview) = h
        .call("GET", &format!("{path}/content"), Some(&admin.token), None)
        .await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["interfaces"], 2);
    assert_eq!(preview["folders"], 1);
    assert_eq!(preview["curation_results"], 5);
    assert_eq!(
        h.call(
            "POST",
            &format!("{path}/clear"),
            Some(&admin.token),
            Some(json!({"confirmation":"错误项目"}))
        )
        .await
        .0,
        400
    );
    assert_eq!(h.endpoint_count(p).await, 2);
    let (_, mcp) = h
        .databases
        .access
        .issue_mcp_token(admin.principal.user_id, 1)
        .await
        .unwrap();
    let (status, data) = h
        .call(
            "POST",
            &format!("{path}/clear"),
            Some(&admin.token),
            Some(json!({"confirmation":"云售后"})),
        )
        .await;
    assert_eq!(status, 200, "{data}");
    assert_eq!(h.endpoint_count(p).await, 0);
    assert_eq!(h.endpoint_count(other).await, 2);
    assert_eq!(h.endpoint_count(child.project_id).await, 2);
    for table in [
        "knowledge.folders",
        "knowledge.placements",
        "knowledge.endpoint_notes",
        "knowledge.field_notes",
        "knowledge.links",
        "knowledge.rounds",
        "knowledge.candidates",
        "knowledge.field_value_archives",
    ] {
        assert_eq!(count(&h, table, p).await, 0, "{table}");
    }
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM observe.fingerprints WHERE endpoint_id=ANY($1)")
            .bind(&ids)
            .fetch_one(&h.sql)
            .await
            .unwrap();
    assert_eq!(retained, 0);
    let outbox: i64 =
        sqlx::query_scalar("SELECT count(*) FROM observe.outbox WHERE event->>'project_id'=$1")
            .bind(p.to_string())
            .fetch_one(&h.sql)
            .await
            .unwrap();
    assert_eq!(outbox, 0);
    assert_eq!(
        h.store
            .list_environments(&admin.principal, p, 1, 100)
            .await
            .unwrap()
            .items[0]
            .name,
        "灰度环境"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM access.environment_sites WHERE environment_id=$1"
        )
        .bind(uuid(env.id))
        .fetch_one(&h.sql)
        .await
        .unwrap(),
        config_before
    );
    assert!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM observe.service_addresses WHERE project_id=$1 AND manual_verdict='own' AND calls=0").bind(uuid(p)).fetch_one(&h.sql).await.unwrap()>0);
    let search = h
        .mcp(
            &mcp,
            "search_interfaces",
            json!({"project_id":p,"query":"order"}),
        )
        .await;
    assert_eq!(search["total"], 0);
    let read = h
        .mcp(
            &mcp,
            "read_interface",
            json!({"project_id":p,"endpoint_id":ids[0],"section":"overview"}),
        )
        .await;
    assert!(read.get("error").is_some());
    let service = nexofolio_knowledge::Knowledge::new(
        h.databases.knowledge.clone(),
        nexofolio_observe::Observe::new(h.databases.observe.clone()),
    );
    assert!(service.candidate(p, candidate).await.is_err());
    assert!(
        service
            .recover_candidate(p, candidate, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        h.call("POST", "/v1/collect/batches", None, Some(batch.clone()))
            .await
            .0,
        200
    );
    assert_eq!(h.endpoint_count(p).await, 0);
    let mut queued = batch.clone();
    queued["batch_id"] = json!(uuid::Uuid::new_v4());
    assert_eq!(
        h.call("POST", "/v1/collect/batches", None, Some(queued))
            .await
            .0,
        200
    );
    assert_eq!(h.endpoint_count(p).await, 0);
    let mut fresh = batch;
    fresh["batch_id"] = json!(uuid::Uuid::new_v4());
    for record in fresh["records"].as_array_mut().unwrap() {
        record["observed_at"] = json!(chrono::Utc::now().to_rfc3339());
    }
    assert_eq!(
        h.call("POST", "/v1/collect/batches", None, Some(fresh))
            .await
            .0,
        200
    );
    assert_eq!(h.endpoint_count(p).await, 2);
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn active_curations_and_concurrent_writes_refuse_clear_and_failure_rolls_back_all_modules() {
    let (h, admin, p, _env, _batch, _ids, candidate) = setup().await;
    let clear = format!("/v1/project-management/projects/{p}/clear");
    let body = json!({"confirmation":"云售后"});
    sqlx::query("UPDATE knowledge.candidates SET state='running' WHERE id=$1")
        .bind(candidate)
        .execute(&h.sql)
        .await
        .unwrap();
    assert_eq!(
        h.call("POST", &clear, Some(&admin.token), Some(body.clone()))
            .await
            .0,
        409
    );
    assert_eq!(h.endpoint_count(p).await, 2);
    let service = nexofolio_knowledge::Knowledge::new(
        h.databases.knowledge.clone(),
        nexofolio_observe::Observe::new(h.databases.observe.clone()),
    );
    service.cancel_candidate(p, candidate).await.unwrap();
    let guard = h.store.activity(p).await.unwrap();
    assert_eq!(
        h.call("POST", &clear, Some(&admin.token), Some(body.clone()))
            .await
            .0,
        409
    );
    drop(guard);
    let mut lease = h
        .store
        .begin_project_reset(&admin.principal, p, None)
        .await
        .unwrap();
    let blocked = h
        .call(
            "POST",
            &format!("/v1/projects/{p}/environments"),
            Some(&admin.token),
            Some(json!({"name":"清空期间禁止写入"})),
        )
        .await;
    assert_eq!(blocked.0, 409);
    let encoded = p.to_string().replace('-', "%2D");
    assert_eq!(
        h.call(
            "POST",
            &format!("/v1/projects/{encoded}/environments"),
            Some(&admin.token),
            Some(json!({"name":"编码路径也禁止写入"}))
        )
        .await
        .0,
        409
    );
    // Lock is transaction-owned and disappears on rollback/cancellation.
    h.databases
        .knowledge
        .prepare_project_reset(lease.connection(), p)
        .await
        .unwrap();
    drop(lease);
    let function = format!("fail_reset_{}", uuid(p).simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE FUNCTION observe.{function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic reset failure'; END $$"))).execute(&h.sql).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE TRIGGER {function} BEFORE DELETE ON observe.endpoints FOR EACH ROW WHEN(OLD.project_id='{}'::uuid) EXECUTE FUNCTION observe.{function}()",p))).execute(&h.sql).await.unwrap();
    assert_eq!(
        h.call("POST", &clear, Some(&admin.token), Some(body.clone()))
            .await
            .0,
        503
    );
    assert_eq!(count(&h, "knowledge.candidates", p).await, 5);
    assert_eq!(count(&h, "knowledge.folders", p).await, 1);
    assert_eq!(h.endpoint_count(p).await, 2);
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {function} ON observe.endpoints"
    )))
    .execute(&h.sql)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP FUNCTION observe.{function}()"
    )))
    .execute(&h.sql)
    .await
    .unwrap();
    assert_eq!(
        h.call("POST", &clear, Some(&admin.token), Some(body))
            .await
            .0,
        200
    );
    assert_eq!(h.endpoint_count(p).await, 0);
}
