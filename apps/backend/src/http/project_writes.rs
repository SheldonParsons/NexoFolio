//! Short request activity leases serialize project writes against content resets.
use super::access::{AccessError, authenticate};
use axum::{
    extract::{Path, Request, State, rejection::PathRejection},
    middleware::Next,
    response::Response,
};
use nexofolio_access_contracts::Sessions;
use nexofolio_common::ProjectId;
use nexofolio_contracts::project_lifecycle::ProjectLifecycle;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub(crate) struct ProjectWriteGate {
    pub sessions: Arc<dyn Sessions>,
    pub lifecycle: Arc<dyn ProjectLifecycle>,
}

pub(crate) async fn protect(
    State(state): State<ProjectWriteGate>,
    params: Result<Path<HashMap<String, String>>, PathRejection>,
    request: Request,
    next: Next,
) -> Result<Response, AccessError> {
    if matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Ok(next.run(request).await);
    }
    if !request.uri().path().starts_with("/v1/projects/") {
        return Ok(next.run(request).await);
    }
    // Use the router's decoded parameters, so percent-encoded UUIDs cannot bypass the barrier.
    let project = params.ok().and_then(|Path(params)| {
        params
            .get("project_id")
            .or_else(|| params.get("project"))
            .and_then(|id| id.parse::<ProjectId>().ok())
    });
    let Some(project) = project else {
        return Ok(next.run(request).await);
    };
    authenticate(state.sessions.as_ref(), request.headers()).await?;
    let _activity = state.lifecycle.activity(project).await?;
    Ok(next.run(request).await)
}
