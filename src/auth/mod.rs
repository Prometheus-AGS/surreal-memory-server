//! Verified caller scope for the HTTP boundary (issue #51).
//!
//! UAR/Gate owns identity, membership, grants and revocation. This module only
//! *verifies* the credential Gate issues and enforces what it says; it never
//! decides who may do what on its own.
//!
//! # Contract with UAR/Gate
//!
//! Gate issues a signed JWT (`Authorization: Bearer <token>`) with:
//!
//! | claim     | required | meaning |
//! |-----------|----------|---------|
//! | `iss`     | yes      | must equal `AUTH_JWT_ISSUER` |
//! | `aud`     | yes      | must equal `AUTH_JWT_AUDIENCE` |
//! | `sub`     | yes      | the authenticated principal; binds MCP sessions |
//! | `tenant`  | yes      | tenant id, `[a-z0-9_]{1,48}`; selects the tenant database |
//! | `iat`     | yes      | issue time; the token is refused once older than `AUTH_JWT_MAX_AGE_SECS` |
//! | `exp`     | yes      | expiry |
//! | `ops`     | no       | permitted operations; `"operator"` allows retry/reject/re-key |
//! | `user_id` | no       | restricts the principal to one user's memories (REST memory/search only) |
//!
//! Revocation is expressed through freshness: Gate issues short-lived tokens
//! and stops re-issuing them when a grant is revoked, so a revoked grant stops
//! working within `AUTH_JWT_MAX_AGE_SECS`.
//!
//! Tenant isolation is one SurrealDB database per tenant: the tenant comes only
//! from the verified token, never from request input, and maps to a
//! server-owned namespace (server mode) or directory (embedded mode).

mod access;
mod settings;
mod tenants;

pub use access::{Access, Caller, Tenant, authenticate, secured};
pub use settings::{AuthMode, JwtVerifier, load_auth_mode};
pub use tenants::{TenantRegistry, TenantRuntime, TenantStorageFactory, tenant_config};

use std::fmt;

/// The permission that allows retry, reject and re-key of durable operations.
pub const OPERATOR_OP: &str = "operator";

const MAX_TENANT_LEN: usize = 48;

/// A validated tenant identifier, safe to embed in a namespace or a path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TenantId(String);

impl TenantId {
    pub fn parse(raw: &str) -> Result<Self, AuthError> {
        let valid = !raw.is_empty()
            && raw.len() <= MAX_TENANT_LEN
            && raw
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if valid {
            Ok(Self(raw.to_string()))
        } else {
            Err(AuthError::Invalid("tenant claim is not a valid tenant id"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The verified caller. Built only from a verified token, or explicitly as the
/// trusted local principal when the server runs in loopback-only local mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub tenant: TenantId,
    pub subject: String,
    pub operator: bool,
    /// When set, the principal may only see and change this user's memories.
    pub user_scope: Option<String>,
}

impl Principal {
    /// The trusted client of a loopback-only server (`AUTH_MODE=local`).
    pub fn local() -> Self {
        Self {
            tenant: TenantId("local".to_string()),
            subject: "local".to_string(),
            operator: true,
            user_scope: None,
        }
    }

    /// The `user_id` a request may act on.
    ///
    /// An unrestricted principal passes the request through. A restricted one
    /// gets its own user id when the request names none, and is refused when
    /// the request names any other value (including the empty string).
    pub fn effective_user(&self, requested: Option<&str>) -> Result<Option<String>, ScopeError> {
        match (&self.user_scope, requested) {
            (None, requested) => Ok(requested.map(str::to_string)),
            (Some(own), None) => Ok(Some(own.clone())),
            (Some(own), Some(asked)) if asked == own => Ok(Some(own.clone())),
            (Some(_), Some(_)) => Err(ScopeError),
        }
    }

    /// Whether this principal may see a record owned by `owner`.
    pub fn may_access(&self, owner: Option<&str>) -> bool {
        match &self.user_scope {
            None => true,
            Some(own) => owner == Some(own.as_str()),
        }
    }

    /// The key an MCP session is bound to.
    pub fn session_owner(&self) -> (TenantId, String) {
        (self.tenant.clone(), self.subject.clone())
    }
}

/// Why a credential was refused. Messages are deliberately generic: they go to
/// the client, and detail belongs in the server log only.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer credential")]
    Missing,
    #[error("invalid credential: {0}")]
    Invalid(&'static str),
}

/// The request named an owner outside the principal's allowed scope.
#[derive(Debug, thiserror::Error)]
#[error("request names an owner outside the credential's scope")]
pub struct ScopeError;
