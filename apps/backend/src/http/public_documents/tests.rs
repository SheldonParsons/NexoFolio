mod fixture;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use fixture::*;
use nexofolio_contracts::endpoint::ServiceAddresses;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tower::ServiceExt;

async fn request(
    f: &Fixture,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25")
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = f
        .app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cache = response
        .headers()
        .get("cache-control")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cache,
    )
}
#[tokio::test]
async fn visitors_read_only_explicit_formal_document_fields() {
    let f = setup().await;
    let project = f.access.project;
    for token in [None, Some("outsider"), Some("expired")] {
        let (status, index, cache) = request(
            &f,
            "GET",
            &format!("/v1/public/projects/{project}/document"),
            token,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache, "no-store");
        assert_eq!(index["project"]["name"], "公开项目");
        assert_eq!(index["entries"][0]["name"], "创建订单");
        assert!(index["catalog"]["folders"][0].get("author").is_none());
        let (status, detail, _) = request(
            &f,
            "GET",
            &format!(
                "/v1/public/projects/{project}/endpoints/{}?environment_id={}",
                f.target, f.access.env
            ),
            token,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["description"], "正式接口说明");
        assert_eq!(detail["fields"][0]["name"], "客户编号");
        assert_eq!(detail["fields"][0]["description"], "客户的业务标识");
        assert!(detail.get("requests").is_none());
        assert!(detail.get("knowledge").is_none());
        assert!(!format!("{index}{detail}").contains("PRIVATE_"));
        assert!(!format!("{index}{detail}").contains(&f.access.member.to_string()));
    }
    assert_eq!(f.reader.raw_reads.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn public_routes_keep_scope_alias_and_read_only_boundaries() {
    let f = setup().await;
    let project = f.access.project;
    for path in [
        format!("/v1/public/projects/{project}/endpoints/{}", f.foreign),
        format!(
            "/v1/public/projects/{project}/endpoints/{}?environment_id={}",
            f.target,
            nexofolio_common::EnvironmentId::new()
        ),
        format!(
            "/v1/public/projects/{}/document",
            nexofolio_common::ProjectId::new()
        ),
        format!(
            "/v1/public/projects/{project}/endpoints/{}/examples/private-sample",
            f.target
        ),
    ] {
        assert_eq!(
            request(&f, "GET", &path, None, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, value, _) = request(
        &f,
        "GET",
        &format!("/v1/public/projects/{project}/endpoints/{}", f.alias),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["endpoint_id"], json!(f.target));
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        assert_eq!(
            request(
                &f,
                method,
                &format!("/v1/public/projects/{project}/document"),
                None,
                json!({})
            )
            .await
            .0,
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
#[tokio::test]
async fn public_read_does_not_unlock_raw_samples_or_member_writes() {
    let f = setup().await;
    let project = f.access.project;
    let sample = format!(
        "/v1/projects/{project}/endpoints/{}/examples/private-sample",
        f.target
    );
    let write = format!("/v1/projects/{project}/service-addresses");
    let body = json!({"address":"https://synthetic.example","verdict":"own"});
    assert_eq!(
        request(&f, "GET", &sample, None, Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&f, "GET", &sample, Some("outsider"), Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for (token, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("outsider"), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            request(&f, "PUT", &write, token, body.clone()).await.0,
            status
        );
    }
    assert!(f.addresses.list(project).await.unwrap().is_empty());
    let (status, raw, _) = request(&f, "GET", &sample, Some("member"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(raw.to_string().contains("PRIVATE_BODY"));
    assert_eq!(
        request(&f, "PUT", &write, Some("member"), body).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(f.addresses.list(project).await.unwrap().len(), 1);
}
#[tokio::test]
async fn mcp_still_requires_identity_and_project_permission_after_public_reading() {
    let f = setup().await;
    let project = f.access.project;
    assert_eq!(
        request(
            &f,
            "GET",
            &format!("/v1/public/projects/{project}/document"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_interface","arguments":{"project_id":project,"endpoint_id":f.target,"section":"overview"}}});
    assert_eq!(
        request(&f, "POST", "/mcp", None, body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, response, _) = request(&f, "POST", "/mcp", Some("outsider"), body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response["result"]["structuredContent"]["error"],
        "PROJECT_ACCESS_DENIED"
    );
    let (_, response, _) = request(&f, "POST", "/mcp", Some("member"), body).await;
    assert_eq!(
        response["result"]["structuredContent"]["interface"]["name"],
        "创建订单"
    );
}
