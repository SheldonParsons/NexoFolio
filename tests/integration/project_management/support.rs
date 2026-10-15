use axum::{Router, body::Body, extract::Request};
use http_body_util::BodyExt;
use nexofolio_access_adapter::PostgresAccess;
use nexofolio_access_contracts::{ExternalIdentity, LoginStore, SessionPrincipal};
use nexofolio_backend::{
    http,
    wiring::{Config, Databases, build_api, build_navigation},
};
use nexofolio_common::{ProjectId, Secret};
use serde_json::{Value, json};
use sqlx::{PgPool, Row, postgres::PgConnectOptions};
use std::{str::FromStr, sync::Arc};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
pub fn uuid(id: impl ToString) -> uuid::Uuid {
    id.to_string().parse().unwrap()
}
pub struct User {
    pub principal: SessionPrincipal,
    pub token: Secret,
}
pub struct Harness {
    pub databases: Databases,
    pub store: PostgresAccess,
    pub sql: PgPool,
    pub app: Router,
    pub instance: String,
}
impl Harness {
    pub async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("isolated TEST_DATABASE_URL");
        let options = PgConnectOptions::from_str(&url).unwrap();
        assert!(
            options
                .get_database()
                .unwrap_or_default()
                .starts_with("nexofolio_access_test_")
        );
        let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let instance = format!("http://{}/api/v1", unused.local_addr().unwrap());
        let config = Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some(url.clone()),
            "NEXOFOLIO_ZENTAO_BASE_URL" => Some(instance.clone()),
            "NEXOFOLIO_SESSION_KEY" => Some("ab".repeat(32)),
            _ => None,
        })
        .unwrap();
        let databases = Databases::new(&config).unwrap();
        databases.migrate().await.unwrap();
        let store =
            PostgresAccess::new(databases.access.clone(), &Secret::new("ab".repeat(32))).unwrap();
        let api = build_api(&config, &databases).unwrap();
        let app = http::router_with_navigation(
            &config,
            Arc::new(databases.access.clone()),
            Arc::new(databases.access.clone()),
            CancellationToken::new(),
            api,
            Some(build_navigation(&databases)),
        );
        let sql = PgPool::connect_with(options).await.unwrap();
        Self {
            databases,
            store,
            sql,
            app,
            instance,
        }
    }
    pub async fn user(&self, account: &str) -> User {
        let session = self
            .store
            .normal_login(&ExternalIdentity {
                instance: self.instance.clone(),
                external_id: uuid::Uuid::new_v4().to_string(),
                account: account.into(),
                display_name: account.into(),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE access.users SET grants_synced=true WHERE id=$1")
            .bind(uuid(session.user.id))
            .execute(&self.sql)
            .await
            .unwrap();
        User {
            principal: SessionPrincipal {
                user_id: session.user.id,
                instance: self.instance.clone(),
            },
            token: session.token,
        }
    }
    pub async fn project(&self, name: &str) -> ProjectId {
        let id = ProjectId::new();
        sqlx::query("INSERT INTO access.projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,$2,$3,$4,'doing',1)").bind(uuid(id)).bind(&self.instance).bind(id.to_string()).bind(name).execute(&self.sql).await.unwrap();
        id
    }
    pub async fn grant(&self, user: &User, project: ProjectId) {
        sqlx::query("INSERT INTO access.user_project_access(user_id,project_id) VALUES($1,$2)")
            .bind(uuid(user.principal.user_id))
            .bind(uuid(project))
            .execute(&self.sql)
            .await
            .unwrap();
    }
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&Secret>,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {}", token.expose()));
        }
        if path == "/mcp" {
            request = request
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-11-25");
        }
        let body = if let Some(body) = body {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        } else {
            Body::empty()
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"non_json": String::from_utf8_lossy(&bytes).chars().take(256).collect::<String>()})),
        )
    }
    pub async fn mcp(&self, token: &Secret, name: &str, args: Value) -> Value {
        let (status,data)=self.call("POST","/mcp",Some(token),Some(json!({"jsonrpc":"2.0","id":uuid::Uuid::new_v4().to_string(),"method":"tools/call","params":{"name":name,"arguments":args}}))).await;
        assert_eq!(status, 200, "{data}");
        data["result"]["structuredContent"].clone()
    }
    pub async fn endpoint_count(&self, project: ProjectId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM observe.endpoints WHERE project_id=$1")
            .bind(uuid(project))
            .fetch_one(&self.sql)
            .await
            .unwrap()
    }
    pub async fn populate(
        &self,
        project: ProjectId,
        environment: impl ToString,
    ) -> (Value, Vec<uuid::Uuid>) {
        let mut batch: Value = serde_json::from_str(include_str!(
            "../../../contracts/collect/v1/fixtures/batch/valid/fetcher-exchanges.json"
        ))
        .unwrap();
        // A synthetic API producer keeps lifecycle tests independent of the live Fetcher release gate.
        batch["platform"] = json!("lifecycle-fixture");
        batch["batch_id"] = json!(uuid::Uuid::new_v4());
        batch["target"]["project_id"] = json!(project);
        batch["target"]["environment"]["id"] = json!(environment.to_string());
        let (status, data) = self
            .call("POST", "/v1/collect/batches", None, Some(batch.clone()))
            .await;
        assert_eq!(status, 200, "{data}");
        assert_eq!(data["accepted"], 2);
        let rows = sqlx::query("SELECT id FROM observe.endpoints WHERE project_id=$1 ORDER BY id")
            .bind(uuid(project))
            .fetch_all(&self.sql)
            .await
            .unwrap();
        (batch, rows.iter().map(|r| r.get("id")).collect())
    }
}
