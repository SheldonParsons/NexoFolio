//! `PostgresAccess` against the shared scope conformance suites.
use async_trait::async_trait;
use nexofolio_access_adapter::{Postgres, PostgresAccess};
use nexofolio_common::{EnvironmentId, ProjectId, Secret};
use nexofolio_contracts::scope::{
    CollectTarget, EnvironmentSelector, ResolvedTarget, ScopeError, SiteBinding, SiteRegistry,
    SiteScope, TargetResolver,
};
use nexofolio_contracts::testing::{
    ScopeFixture, site_registry_conformance, target_resolver_conformance,
};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

/// The store under test plus direct table access for seeding.
struct Harness {
    store: PostgresAccess,
    pool: PgPool,
}

impl Harness {
    async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("set an isolated TEST_DATABASE_URL");
        let database = Postgres::new(&Secret::new(&url), 4, Duration::from_secs(2)).unwrap();
        database.migrate().await.unwrap();
        let options = PgConnectOptions::from_str(&url)
            .unwrap()
            .options([("search_path", "access")]);
        Self {
            store: PostgresAccess::new(database, &Secret::new("ab".repeat(32))).unwrap(),
            pool: PgPool::connect_with(options).await.unwrap(),
        }
    }
}

fn uuid(id: impl ToString) -> Uuid {
    id.to_string().parse().unwrap()
}

#[async_trait]
impl ScopeFixture for Harness {
    async fn create_project(&self) -> ProjectId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,'https://fixture.test',$2,'fixture','active',1)")
            .bind(id)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .unwrap();
        id.to_string().parse().unwrap()
    }

    async fn create_environment(&self, project: ProjectId, name: &str) -> EnvironmentId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,$3)")
            .bind(id)
            .bind(uuid(project))
            .bind(name)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO environment_names(project_id,name,environment_id) VALUES($1,$2,$3)",
        )
        .bind(uuid(project))
        .bind(name)
        .bind(id)
        .execute(&self.pool)
        .await
        .unwrap();
        id.to_string().parse().unwrap()
    }

    async fn environment_names(&self, project: ProjectId) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM environments WHERE project_id=$1 ORDER BY name")
            .bind(uuid(project))
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }
}

#[async_trait]
impl TargetResolver for Harness {
    async fn resolve(&self, target: &CollectTarget) -> Result<ResolvedTarget, ScopeError> {
        self.store.resolve(target).await
    }
}

#[async_trait]
impl SiteRegistry for Harness {
    async fn lookup(&self, page_url: &str) -> Result<Option<SiteBinding>, ScopeError> {
        self.store.lookup(page_url).await
    }
    async fn bind(&self, binding: SiteBinding) -> Result<(), ScopeError> {
        self.store.bind(binding).await
    }
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; run with --ignored explicitly"]
async fn postgres_passes_target_resolver_conformance() {
    target_resolver_conformance(&Harness::new().await).await;
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; run with --ignored explicitly"]
async fn postgres_passes_site_registry_conformance() {
    site_registry_conformance(&Harness::new().await).await;
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; run with --ignored explicitly"]
async fn sites_annotate_an_environment_once() {
    let harness = Harness::new().await;
    let project = harness.create_project().await;
    let target = CollectTarget {
        project_id: project,
        environment: Some(EnvironmentSelector::Name("测试环境".into())),
        site: Some(SiteScope::new("https://Shop.example.test", "/").unwrap()),
        source_url: None,
    };
    let first = harness.resolve(&target).await.unwrap();
    let again = harness.resolve(&target).await.unwrap();
    assert_eq!(first, again);
    let sites: Vec<(String, String)> =
        sqlx::query_as("SELECT origin,prefix FROM environment_sites WHERE environment_id=$1")
            .bind(uuid(first.environment_id.unwrap()))
            .fetch_all(&harness.pool)
            .await
            .unwrap();
    assert_eq!(
        sites,
        vec![("https://shop.example.test".into(), "/".into())]
    );
}
