use super::*;
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::FieldLocation, knowledge::LinkContext};
use sqlx::Connection;

#[tokio::test]
#[ignore = "needs isolated TEST_DATABASE_URL"]
async fn upgrade_old_index_keeps_unknown_legacy_identity_and_allows_qualified_rows() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let mut db = sqlx::PgConnection::connect(url.expose()).await.unwrap();
    // A session-local table models the already-populated old index without
    // changing the real knowledge schema used by other tests.
    sqlx::raw_sql("CREATE TEMP TABLE links (
        from_endpoint uuid NOT NULL, to_endpoint uuid NOT NULL, relation text NOT NULL,
        from_field text, to_field text, context jsonb NOT NULL DEFAULT '{}');
        CREATE UNIQUE INDEX links_identity ON links
        (from_endpoint,to_endpoint,relation,coalesce(from_field,''),coalesce(to_field,''),coalesce(context->>'environment',''));
        INSERT INTO links VALUES ('00000000-0000-0000-0000-000000000001',
            '00000000-0000-0000-0000-000000000002','parameter_source','data.id','id','{}');")
        .execute(&mut db).await.unwrap();
    let migration = include_str!("../../migrations/202610070002_link_field_identity.sql").replace(
        "DROP INDEX links_identity",
        "DROP INDEX pg_temp.links_identity",
    );
    // Only a compile-time migration and a fixed schema qualification above;
    // no user data enters this SQL text.
    sqlx::raw_sql(sqlx::AssertSqlSafe(migration.as_str()))
        .execute(&mut db)
        .await
        .unwrap();
    for location in [FieldLocation::Path, FieldLocation::Query] {
        let context =
            json!({"from_location":FieldLocation::ResponseBody{status:200},"to_location":location});
        sqlx::query(
            "INSERT INTO links VALUES ('00000000-0000-0000-0000-000000000001',
            '00000000-0000-0000-0000-000000000002','parameter_source','data.id','id',$1)",
        )
        .bind(context)
        .execute(&mut db)
        .await
        .unwrap();
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM links")
        .fetch_one(&mut db)
        .await
        .unwrap();
    assert_eq!(count, 3);
    let old: i64 = sqlx::query_scalar("SELECT count(*) FROM links WHERE context='{}'")
        .fetch_one(&mut db)
        .await
        .unwrap();
    assert_eq!(
        old, 1,
        "unknown legacy position is preserved without inventing a location"
    );
}

#[tokio::test]
#[ignore = "needs isolated TEST_DATABASE_URL"]
async fn qualified_locations_statuses_environments_and_legacy_rows_survive_refresh_and_restore() {
    let store = store().await.expect("TEST_DATABASE_URL required");
    let project = ProjectId::new();
    let env = EnvironmentId::new();
    let base = Link {
        id: LinkId::new(),
        from: EndpointId::new(),
        to: EndpointId::new(),
        from_field: Some("data.id".into()),
        to_field: Some("id".into()),
        relation: Relation::ParameterSource,
        evidence: "legacy".into(),
        context: LinkContext {
            environment: Some(env),
            ..Default::default()
        },
        author: curate(),
        at: at(),
    };
    let mut links = vec![base.clone()];
    for (status, location, environment) in [
        (200, FieldLocation::Path, env),
        (200, FieldLocation::Query, env),
        (400, FieldLocation::Path, env),
        (200, FieldLocation::Path, EnvironmentId::new()),
    ] {
        links.push(Link {
            id: LinkId::new(),
            context: LinkContext {
                from_location: Some(FieldLocation::ResponseBody { status }),
                to_location: Some(location),
                environment: Some(environment),
                ..Default::default()
            },
            ..base.clone()
        });
    }
    let mut tx = store.begin().await.unwrap();
    tx.lock_project(project).await.unwrap();
    for link in &links {
        tx.put_link(project, link).await.unwrap();
    }
    assert_eq!(tx.links(project).await.unwrap().len(), 5);
    for link in &links {
        assert_eq!(
            tx.find_link(&link.identity()).await.unwrap().unwrap().id,
            link.id
        );
    }
    let old = &links[2];
    tx.put_link(
        project,
        &Link {
            id: LinkId::new(),
            evidence: "updated query only".into(),
            ..old.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        tx.link(old.id).await.unwrap().unwrap().evidence,
        "updated query only"
    );
    assert_eq!(
        tx.link(links[1].id).await.unwrap().unwrap().evidence,
        "legacy"
    );
    let before = serde_json::to_value(old).unwrap();
    tx.restore(project, &Target::Link { id: old.id }, Some(&before))
        .await
        .unwrap();
    assert_eq!(tx.link(old.id).await.unwrap().unwrap(), *old);
    tx.restore(project, &Target::Link { id: old.id }, None)
        .await
        .unwrap();
    assert_eq!(tx.links(project).await.unwrap().len(), 4);
    tx.restore(project, &Target::Link { id: old.id }, Some(&before))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = store.begin().await.unwrap();
    assert_eq!(tx.links(project).await.unwrap().len(), 5);
    assert_eq!(tx.find_link(&old.identity()).await.unwrap().unwrap(), *old);
}
