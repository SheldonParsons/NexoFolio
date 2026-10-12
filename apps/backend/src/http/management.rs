//! Shared server gate for private management routes, before request extraction.
use super::access::{AccessError, authenticate};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use nexofolio_access_contracts::{Sessions, can_manage_projects};
use nexofolio_common::Error;
use std::sync::Arc;

pub(crate) async fn administrator(
    State(sessions): State<Arc<dyn Sessions>>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers().clone();
    let principal = async {
        let principal = authenticate(sessions.as_ref(), &headers).await?;
        let user = sessions.me(&principal).await?;
        if !can_manage_projects(&user.account, user.enabled) {
            return Err(Error::NotFound);
        }
        Ok::<_, Error>(principal)
    }
    .await;
    match principal {
        Ok(principal) => {
            if let Some(audit) = request.extensions().get::<crate::audit::Context>() {
                audit.identify(principal.user_id);
            }
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(_) => AccessError(Error::NotFound).into_response(),
    }
}
