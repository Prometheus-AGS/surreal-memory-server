//! Request-time enforcement: the authenticating middleware and the extractors
//! that hand handlers a tenant-scoped state.

use axum::{
    Json,
    extract::{FromRequestParts, Request, State},
    http::{HeaderValue, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use super::{JwtVerifier, Principal, TenantRegistry};
use crate::api::{ApiError, ApiFailure, AppState};

/// Paths that need no credential and expose only non-sensitive status.
const PUBLIC_PATHS: [&str; 2] = ["/health", "/ready"];

/// Path prefixes a user-restricted principal may use. Everything else is
/// refused because those routes cannot yet honour a per-user restriction;
/// a new route is therefore denied to restricted principals until listed here.
const USER_SCOPED_PREFIXES: [&str; 2] = ["/api/v1/memory", "/api/v1/search"];

const MAX_TOKEN_BYTES: usize = 8 * 1024;

/// How requests are authorised, carried on the router state.
#[derive(Clone)]
pub enum Access {
    /// Loopback-only trusted client; one storage, no credentials.
    Local,
    /// Verified Gate tokens; storage is resolved per tenant.
    Jwt {
        verifier: Arc<JwtVerifier>,
        tenants: Arc<TenantRegistry>,
    },
}

impl AppState {
    /// The state a request from `principal` must use. In JWT mode this is the
    /// tenant's own storage and ledger; the base storage is never returned.
    pub async fn for_principal(&self, principal: &Principal) -> Result<AppState, ApiFailure> {
        match &self.access {
            Access::Local => Ok(self.clone()),
            Access::Jwt { tenants, .. } => {
                let runtime = tenants.runtime(&principal.tenant).await.map_err(|error| {
                    tracing::error!(tenant = %principal.tenant, error = %format!("{error:#}"), "tenant resolution failed");
                    failure(StatusCode::SERVICE_UNAVAILABLE, "tenant storage unavailable")
                })?;
                Ok(AppState {
                    storage: runtime.storage,
                    operations: runtime.operations,
                    embedding_service: Arc::clone(&self.embedding_service),
                    access: self.access.clone(),
                })
            }
        }
    }
}

fn failure(status: StatusCode, message: &str) -> ApiFailure {
    (
        status,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
}

fn unauthorized() -> Response {
    let mut response = failure(StatusCode::UNAUTHORIZED, "authentication required").into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn bearer_token(request: &Request) -> Option<&str> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() && token.len() <= MAX_TOKEN_BYTES)
        .then_some(token.trim())
}

fn is_public(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
}

fn allowed_for_user_scope(path: &str) -> bool {
    USER_SCOPED_PREFIXES.iter().any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Authenticate every request before any handler or storage runs.
///
/// Default-deny: only [`PUBLIC_PATHS`] skip verification, so a route added
/// later is protected without being listed anywhere.
pub async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let verifier = match &state.access {
        Access::Local => {
            request.extensions_mut().insert(Principal::local());
            return next.run(request).await;
        }
        Access::Jwt { verifier, .. } => verifier,
    };

    if is_public(request.uri().path()) {
        return next.run(request).await;
    }

    let Some(token) = bearer_token(&request) else {
        return unauthorized();
    };
    let principal = match verifier.verify(token) {
        Ok(principal) => principal,
        Err(error) => {
            tracing::warn!(%error, path = request.uri().path(), "request refused");
            return unauthorized();
        }
    };

    if principal.user_scope.is_some() && !allowed_for_user_scope(request.uri().path()) {
        return failure(
            StatusCode::FORBIDDEN,
            "this credential is restricted to the memory and search routes",
        )
        .into_response();
    }

    request.extensions_mut().insert(principal);
    next.run(request).await
}

/// Mount `routes` on `state` behind the authenticating middleware, the way the
/// full router does. Tests and embedders that mount a sub-router use this so the
/// handlers still receive a verified caller.
pub fn secured(routes: axum::Router<AppState>, state: AppState) -> axum::Router {
    routes
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            authenticate,
        ))
        .with_state(state)
}

/// The verified caller of this request.
pub struct Caller(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Only the middleware inserts a Principal. Absent means the request
        // never passed authentication, so fail closed.
        parts
            .extensions
            .get::<Principal>()
            .cloned()
            .map(Caller)
            .ok_or_else(|| failure(StatusCode::UNAUTHORIZED, "authentication required"))
    }
}

/// The tenant-scoped application state for this request. Handlers take this
/// where they previously took `State<AppState>`.
pub struct Tenant(pub AppState);

impl FromRequestParts<AppState> for Tenant {
    type Rejection = ApiFailure;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Caller(principal) = Caller::from_request_parts(parts, state).await?;
        state.for_principal(&principal).await.map(Tenant)
    }
}
