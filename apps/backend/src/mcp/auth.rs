use crate::http::RequestId;
use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use nexofolio_access_contracts::{McpPrincipal, McpTokenVerifier};
use nexofolio_common::{Error, Secret};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct McpRequestContext {
    pub request_id: String,
    pub principal: McpPrincipal,
}

/// Applied to the entire MCP transport, not just tools/call.
pub async fn require_token(
    State(verifier): State<Arc<dyn McpTokenVerifier>>,
    mut request: Request,
    next: Next,
) -> Response {
    let denied = || {
        (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"nexofolio-mcp\"")],
        Json(serde_json::json!({"error":{"code":"UNAUTHENTICATED","message":"A valid MCP token is required"}})),
    ).into_response()
    };
    let values = request.headers().get_all(header::AUTHORIZATION);
    let mut values = values.iter();
    let Some(value) = values.next().and_then(|v| v.to_str().ok()) else {
        return denied();
    };
    if values.next().is_some() {
        return denied();
    }
    let Some((scheme, value)) = value.split_once(' ') else {
        return denied();
    };
    if !scheme.eq_ignore_ascii_case("bearer")
        || value.is_empty()
        || value.len() > 4096
        || value.bytes().any(|b| b.is_ascii_whitespace())
    {
        return denied();
    }
    let secret = Secret::new(value);
    match verifier.verify(&secret).await {
        Ok(principal) => {
            let request_id = request.extensions().get::<RequestId>().map(|id| id.0.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            request.extensions_mut().insert(McpRequestContext { request_id, principal });
            next.run(request).await
        }
        Err(Error::Unavailable { .. }) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"AUTH_UNAVAILABLE","message":"Authentication is temporarily unavailable"}})),
        ).into_response(),
        Err(_) => denied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{Router, body::Body, extract::Extension, http::Request, middleware, routing::get};
    use nexofolio_common::{Result, TokenId, UserId};
    use tower::ServiceExt;

    struct FixtureVerifier(McpPrincipal);
    #[async_trait]
    impl McpTokenVerifier for FixtureVerifier {
        async fn verify(&self, _: &Secret) -> Result<McpPrincipal> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn validated_principal_and_request_id_reach_handler_extensions() {
        let user_id = UserId::new();
        let verifier: Arc<dyn McpTokenVerifier> = Arc::new(FixtureVerifier(McpPrincipal {
            user_id,
            token_id: TokenId::new(),
        }));
        let app = Router::new()
            .route(
                "/",
                get(
                    move |Extension(ctx): Extension<McpRequestContext>| async move {
                        assert_eq!(ctx.principal.user_id, user_id);
                        assert_eq!(ctx.request_id, "test-request");
                        StatusCode::NO_CONTENT
                    },
                ),
            )
            .layer(middleware::from_fn_with_state(verifier, require_token));
        let mut request = Request::builder()
            .uri("/")
            .header("authorization", "Bearer fixture-only")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(RequestId("test-request".into()));
        assert_eq!(
            app.oneshot(request).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
    }
}
