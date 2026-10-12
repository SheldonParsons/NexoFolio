use super::support::*;
use nexofolio_audit_contracts::{AuditStore, Event, Limits};
use nexofolio_contracts::knowledge::{CandidateState, KnowledgeCandidates};
use serde_json::json;
fn event(
    project: impl ToString,
    resource: impl ToString,
    kind: &str,
    route: &str,
    actor: &str,
) -> Event {
    Event {
        at: chrono::Utc::now(),
        kind: kind.into(),
        action: if kind == "mcp" {
            "read_interface"
        } else {
            "page.view"
        }
        .into(),
        actor: actor.into(),
        route: route.into(),
        method: if kind == "page" { "BROWSER" } else { "GET" }.into(),
        status: 200,
        duration_ms: 5,
        request_id: uuid::Uuid::new_v4().to_string(),
        project: project.to_string(),
        resource: resource.to_string(),
        code: "OK".into(),
    }
}
#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn access_analysis_aggregates_all_retained_rows_separates_resources_and_pages_exact_visits() {
    let h = Harness::new().await;
    let admin = h.user("A80646").await;
    let member = h.user("member").await;
    let project = h.project("访问分析合成项目").await;
    let actor = format!("user:{}", member.principal.user_id);
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    let mut events = vec![event(project, a, "page", "document-endpoint", &actor); 250];
    events.push(event(
        project,
        b,
        "page",
        "document-endpoint",
        "ip:127.0.0.1",
    ));
    events.push(event(
        project,
        a,
        "api",
        "/v1/projects/{project}/endpoints/{id}",
        &actor,
    ));
    events.push(event(project, a, "mcp", "/mcp", &actor));
    events.push(event(
        project,
        "",
        "api",
        "/v1/projects/{project}/curate",
        &actor,
    ));
    h.databases
        .audit
        .write(&events, &Limits::default())
        .await
        .unwrap();
    let base = format!("/v1/audit/access?project={project}");
    for token in [None, Some(&member.token)] {
        assert_eq!(
            h.call("GET", &format!("{base}&page=invalid"), token, None)
                .await
                .0,
            404
        )
    }
    let (status, data) = h
        .call(
            "GET",
            &format!("{base}&kind=page&limit=1"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(status, 200, "{data}");
    assert_eq!(data["total"], 2);
    assert_eq!(data["matching_visits"], 251);
    assert_eq!(
        h.call(
            "GET",
            &format!("{base}&kind=page&page_group=docs"),
            Some(&admin.token),
            None
        )
        .await
        .1["matching_visits"],
        251
    );
    assert_eq!(
        h.call(
            "GET",
            &format!("{base}&kind=page&page_group=home"),
            Some(&admin.token),
            None
        )
        .await
        .1["matching_visits"],
        0
    );
    assert_eq!(
        h.call(
            "GET",
            &format!("{base}&kind=api&page_group=docs"),
            Some(&admin.token),
            None
        )
        .await
        .0,
        400
    );

    assert_eq!(data["items"][0]["hits"], 250);
    assert_eq!(data["items"][0]["users"], 1);
    assert_eq!(data["projects"][project.to_string()], "访问分析合成项目");
    let (_, last) = h
        .call(
            "GET",
            &format!("{base}&kind=page&resource={a}&view=visits&page=13&limit=20"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(last["total"], 250);
    assert_eq!(last["items"].as_array().unwrap().len(), 10);
    assert!(last["items"][0]["at"].is_string());
    assert_eq!(last["actors"][&actor], "member");
    assert_eq!(
        h.call("GET", &format!("{base}&kind=api"), Some(&admin.token), None)
            .await
            .1["matching_visits"],
        1
    );
    assert_eq!(
        h.call(
            "GET",
            &format!("{base}&kind=api&background=true"),
            Some(&admin.token),
            None
        )
        .await
        .1["matching_visits"],
        2
    );
    assert_eq!(
        h.call("GET", &format!("{base}&kind=mcp"), Some(&admin.token), None)
            .await
            .1["items"][0]["action"],
        "read_interface"
    );
    sqlx::query("INSERT INTO audit.buckets(key,hour,kind,actor,project,route,method,status,hits,duration_ms,max_duration_ms) VALUES($1,date_trunc('hour',now()-interval '2 days'),'page','unknown',$2,'projects','BROWSER',0,12,0,0)").bind(format!("{}{}",uuid::Uuid::new_v4().simple(),uuid::Uuid::new_v4().simple())).bind(project.to_string()).execute(&h.sql).await.unwrap();
    let historical = h
        .call(
            "GET",
            &format!("{base}&kind=page&view=legacy"),
            Some(&admin.token),
            None,
        )
        .await
        .1;
    assert_eq!(historical["precision"], "hour");
    assert_eq!(historical["items"][0]["hits"], 12);
    assert!(historical["items"][0].get("at").is_none());
    // Supporting queries can be manual or automated; no source attribution is inferred.
    let supporting = [
        event(project, "", "api", "/v1/sites/lookup", &actor),
        event(project, "", "api", "/v1/downloads", &actor),
        event(project, "", "api", "/v1/auth/me", &actor),
    ];
    h.databases
        .audit
        .write(&supporting, &Limits::default())
        .await
        .unwrap();
    assert_eq!(
        h.call("GET", &format!("{base}&kind=api"), Some(&admin.token), None)
            .await
            .1["matching_visits"],
        1
    );
    assert_eq!(
        h.call(
            "GET",
            &format!("{base}&kind=api&background=true"),
            Some(&admin.token),
            None
        )
        .await
        .1["matching_visits"],
        5
    );
    let limits = Limits {
        events: 5,
        ..Limits::default()
    };
    h.databases.audit.write(&[], &limits).await.unwrap();
    let after = h.call("GET", &base, Some(&admin.token), None).await.1;
    assert_eq!(after["coverage"]["stored_details"], 5);
    assert!(after["coverage"]["capacity_evictions"].as_u64().unwrap() >= 249);
    let mut old = event(project, "", "page", "projects", &actor);
    old.at -= chrono::TimeDelta::days(100);
    h.databases
        .audit
        .write(&[old], &Limits::default())
        .await
        .unwrap();
    h.databases.audit.purge(&Limits::default()).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.access_events WHERE at<now()-interval '90 days'",
    )
    .fetch_one(&h.sql)
    .await
    .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn persisted_diagnostic_survives_service_recreation_excludes_samples_and_other_round_events()
{
    let h = Harness::new().await;
    let admin = h.user("a80646").await;
    let member = h.user("other").await;
    let project = h.project("诊断合成项目").await;
    let other = h.project("另一项目").await;
    let service = nexofolio_knowledge::Knowledge::new(
        h.databases.knowledge.clone(),
        nexofolio_observe::Observe::new(h.databases.observe.clone()),
    );
    let c = service.create_candidate(project, None, None).await.unwrap();
    let run = uuid::Uuid::new_v4();
    let start = chrono::Utc::now();
    service.checkpoint_candidate(project,c.id,CandidateState::Interrupted,json!({"status":"failed","run_id":run,"started_at":start,"heartbeat_at":start,"stage":"fields","message":"password=do-not-export","execution_input":{"sample":"BUSINESS_SECRET"},"api_key":"sk-secret","attempts":1,"completed":2,"total":3,"fields_failed":4,"retries":2,"attempt_history":[{"run_id":run,"started_at":start,"ended_at":start,"outcome":"cancelled"}],"issues":[{"endpoint":uuid::Uuid::new_v4(),"stage":"fields","code":"RATE_LIMITED","message":"sk-secret","fields":4,"retryable":true}],"activity":[{"id":1,"at":start,"stage":"fields","kind":"retry","message":"BUSINESS_SECRET","batch":2}]})).await.unwrap();
    let a = event(project, c.id, "operation", "/curate", "unknown");
    let mut b = event(other, c.id, "operation", "/curate", "unknown");
    b.code = "OTHER_PROJECT".into();
    let mut d = event(
        project,
        uuid::Uuid::new_v4(),
        "operation",
        "/curate",
        "unknown",
    );
    d.code = "OTHER_ROUND".into();
    h.databases
        .audit
        .write(&[a, b, d], &Limits::default())
        .await
        .unwrap();
    let path = format!("/v1/audit/curations/{project}/{}", c.id);
    for token in [None, Some(&member.token)] {
        assert_eq!(h.call("GET", &path, token, None).await.0, 404)
    }
    let (status, data) = h.call("GET", &path, Some(&admin.token), None).await;
    assert_eq!(status, 200, "{data}");
    assert_eq!(data["candidate"]["status"], "stopped");
    assert_eq!(data["counts"]["fields_failed"], 4);
    assert_eq!(data["related_events"]["included"], 1);
    assert_eq!(data["issues"]["items"][0]["code"], "RATE_LIMITED");
    for secret in [
        "sk-secret",
        "BUSINESS_SECRET",
        "password=do-not-export",
        "OTHER_ROUND",
        "OTHER_PROJECT",
    ] {
        assert!(!data.to_string().contains(secret));
    }
    assert_eq!(
        h.call(
            "GET",
            &format!("/v1/audit/curations/{other}/{}", c.id),
            Some(&admin.token),
            None
        )
        .await
        .0,
        404
    );
    let fresh = nexofolio_knowledge::Knowledge::new(
        h.databases.knowledge.clone(),
        nexofolio_observe::Observe::new(h.databases.observe.clone()),
    );
    assert_eq!(
        fresh
            .candidate_diagnostics(project, c.id)
            .await
            .unwrap()
            .summary
            .progress["run_id"],
        json!(run)
    );
}
