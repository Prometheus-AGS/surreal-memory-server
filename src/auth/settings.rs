//! Authentication mode selection and JWT verification.

use anyhow::{Context, Result, bail};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AuthError, OPERATOR_OP, Principal, TenantId};

const DEFAULT_MAX_AGE_SECS: u64 = 300;
const CLOCK_LEEWAY_SECS: u64 = 5;
const MIN_HMAC_KEY_BYTES: usize = 32;

/// How the HTTP boundary identifies callers.
#[derive(Clone)]
pub enum AuthMode {
    /// No credentials. Accepted only on a loopback bind.
    Local,
    /// Every request except health/readiness carries a verified Gate token.
    Jwt(Arc<JwtVerifier>),
}

impl AuthMode {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    tenant: String,
    iat: u64,
    exp: u64,
    #[serde(default)]
    ops: Vec<String>,
    #[serde(default)]
    user_id: Option<String>,
}

pub struct JwtVerifier {
    key: DecodingKey,
    validation: Validation,
    max_age_secs: u64,
}

impl JwtVerifier {
    pub fn new(
        algorithm: Algorithm,
        key: DecodingKey,
        issuer: &str,
        audience: &str,
        max_age_secs: u64,
    ) -> Self {
        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        // Expiry is checked in `verify_at` against the same `now` as freshness,
        // so both use one clock reading.
        validation.validate_exp = false;
        validation.leeway = CLOCK_LEEWAY_SECS;
        Self {
            key,
            validation,
            max_age_secs,
        }
    }

    /// Verify a bearer token against the wall clock.
    pub fn verify(&self, token: &str) -> Result<Principal, AuthError> {
        self.verify_at(token, unix_now())
    }

    /// Verify a bearer token as of `now` (seconds since the epoch).
    pub fn verify_at(&self, token: &str, now: u64) -> Result<Principal, AuthError> {
        let data = decode::<Claims>(token, &self.key, &self.validation)
            .map_err(|_| AuthError::Invalid("token rejected"))?;
        let claims = data.claims;

        if now > claims.exp.saturating_add(CLOCK_LEEWAY_SECS) {
            return Err(AuthError::Invalid("token expired"));
        }
        if claims.iat > now + CLOCK_LEEWAY_SECS {
            return Err(AuthError::Invalid("token issued in the future"));
        }
        if now.saturating_sub(claims.iat) > self.max_age_secs {
            return Err(AuthError::Invalid("token is not fresh"));
        }
        if claims.sub.trim().is_empty() {
            return Err(AuthError::Invalid("subject is empty"));
        }
        if claims.user_id.as_deref().is_some_and(str::is_empty) {
            return Err(AuthError::Invalid("user_id claim is empty"));
        }

        Ok(Principal {
            tenant: TenantId::parse(&claims.tenant)?,
            subject: claims.sub,
            operator: claims.ops.iter().any(|op| op == OPERATOR_OP),
            user_scope: claims.user_id,
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Read the authentication mode from the environment, refusing any
/// combination that would publish an unauthenticated backend.
pub fn load_auth_mode(api_host: IpAddr) -> Result<AuthMode> {
    load_auth_mode_from(api_host, &|name| std::env::var(name).ok())
}

pub(crate) fn load_auth_mode_from(
    api_host: IpAddr,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<AuthMode> {
    let mode = lookup("AUTH_MODE").unwrap_or_else(|| "local".to_string());
    match mode.as_str() {
        "local" => {
            if !api_host.is_loopback() {
                bail!(
                    "refusing to start: AUTH_MODE=local serves an unauthenticated API, \
                     but API_HOST={api_host} is not a loopback address. Bind API_HOST to \
                     127.0.0.1, or set AUTH_MODE=jwt with the AUTH_JWT_* settings"
                );
            }
            Ok(AuthMode::Local)
        }
        "jwt" => Ok(AuthMode::Jwt(Arc::new(jwt_verifier_from(lookup)?))),
        other => bail!("AUTH_MODE must be 'local' or 'jwt', got {other:?}"),
    }
}

fn jwt_verifier_from(lookup: &dyn Fn(&str) -> Option<String>) -> Result<JwtVerifier> {
    let required = |name: &str| -> Result<String> {
        lookup(name)
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("AUTH_MODE=jwt requires {name}"))
    };
    let issuer = required("AUTH_JWT_ISSUER")?;
    let audience = required("AUTH_JWT_AUDIENCE")?;
    let algorithm = parse_algorithm(&required("AUTH_JWT_ALGORITHM")?)?;
    let key_material = match lookup("AUTH_JWT_KEY").filter(|value| !value.trim().is_empty()) {
        Some(key) => key,
        None => {
            let path = required("AUTH_JWT_KEY_FILE")
                .context("AUTH_MODE=jwt requires AUTH_JWT_KEY or AUTH_JWT_KEY_FILE")?;
            std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read AUTH_JWT_KEY_FILE {path}"))?
        }
    };
    let max_age_secs = match lookup("AUTH_JWT_MAX_AGE_SECS") {
        Some(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|secs| *secs > 0)
            .context("AUTH_JWT_MAX_AGE_SECS must be a positive integer")?,
        None => DEFAULT_MAX_AGE_SECS,
    };

    let key = decoding_key(algorithm, key_material.trim())?;
    Ok(JwtVerifier::new(
        algorithm,
        key,
        &issuer,
        &audience,
        max_age_secs,
    ))
}

fn parse_algorithm(raw: &str) -> Result<Algorithm> {
    match raw {
        "HS256" => Ok(Algorithm::HS256),
        "RS256" => Ok(Algorithm::RS256),
        "ES256" => Ok(Algorithm::ES256),
        "EdDSA" => Ok(Algorithm::EdDSA),
        other => {
            bail!("AUTH_JWT_ALGORITHM must be one of HS256, RS256, ES256, EdDSA; got {other:?}")
        }
    }
}

fn decoding_key(algorithm: Algorithm, material: &str) -> Result<DecodingKey> {
    match algorithm {
        Algorithm::HS256 => {
            if material.len() < MIN_HMAC_KEY_BYTES {
                bail!("an HS256 AUTH_JWT_KEY must be at least {MIN_HMAC_KEY_BYTES} bytes");
            }
            Ok(DecodingKey::from_secret(material.as_bytes()))
        }
        Algorithm::RS256 => DecodingKey::from_rsa_pem(material.as_bytes())
            .context("AUTH_JWT_KEY is not a valid RSA public key PEM"),
        Algorithm::ES256 => DecodingKey::from_ec_pem(material.as_bytes())
            .context("AUTH_JWT_KEY is not a valid EC public key PEM"),
        _ => DecodingKey::from_ed_pem(material.as_bytes())
            .context("AUTH_JWT_KEY is not a valid Ed25519 public key PEM"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::json;
    use std::collections::HashMap;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    const NOW: u64 = 1_800_000_000;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn jwt_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AUTH_MODE", "jwt"),
            ("AUTH_JWT_ISSUER", "gate"),
            ("AUTH_JWT_AUDIENCE", "memory"),
            ("AUTH_JWT_ALGORITHM", "HS256"),
            ("AUTH_JWT_KEY", SECRET),
        ]
    }

    fn verifier() -> JwtVerifier {
        JwtVerifier::new(
            Algorithm::HS256,
            DecodingKey::from_secret(SECRET.as_bytes()),
            "gate",
            "memory",
            300,
        )
    }

    fn token(overrides: serde_json::Value) -> String {
        let mut claims = json!({
            "iss": "gate", "aud": "memory", "sub": "svc-a", "tenant": "acme",
            "iat": NOW, "exp": NOW + 600,
        });
        for (key, value) in overrides.as_object().cloned().unwrap_or_default() {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(&key);
            } else {
                claims[key] = value;
            }
        }
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn local_mode_on_a_loopback_bind_is_the_safe_default() {
        let mode = load_auth_mode_from("127.0.0.1".parse().unwrap(), &env(&[])).unwrap();
        assert!(mode.is_local());
    }

    #[test]
    fn local_mode_refuses_a_non_loopback_bind() {
        let error = load_auth_mode_from("0.0.0.0".parse().unwrap(), &env(&[]))
            .err()
            .expect("must refuse");
        assert!(error.to_string().contains("not a loopback"));
    }

    #[test]
    fn jwt_mode_refuses_each_missing_setting() {
        for missing in [
            "AUTH_JWT_ISSUER",
            "AUTH_JWT_AUDIENCE",
            "AUTH_JWT_ALGORITHM",
            "AUTH_JWT_KEY",
        ] {
            let pairs: Vec<_> = jwt_env()
                .into_iter()
                .filter(|(k, _)| *k != missing)
                .collect();
            let result = load_auth_mode_from("0.0.0.0".parse().unwrap(), &env(&pairs));
            assert!(result.is_err(), "startup must fail without {missing}");
        }
    }

    #[test]
    fn jwt_mode_refuses_a_weak_hmac_key_and_an_unknown_mode() {
        let mut pairs = jwt_env();
        pairs.retain(|(k, _)| *k != "AUTH_JWT_KEY");
        pairs.push(("AUTH_JWT_KEY", "short"));
        assert!(load_auth_mode_from("0.0.0.0".parse().unwrap(), &env(&pairs)).is_err());
        assert!(
            load_auth_mode_from("127.0.0.1".parse().unwrap(), &env(&[("AUTH_MODE", "off")]))
                .is_err()
        );
    }

    #[test]
    fn jwt_mode_accepts_complete_settings_on_any_bind() {
        let mode = load_auth_mode_from("0.0.0.0".parse().unwrap(), &env(&jwt_env())).unwrap();
        assert!(!mode.is_local());
    }

    #[test]
    fn a_valid_token_yields_the_tenant_subject_and_capabilities() {
        let principal = verifier()
            .verify_at(
                &token(json!({"ops": ["operator"], "user_id": "u1"})),
                NOW + 10,
            )
            .unwrap();
        assert_eq!(principal.tenant.as_str(), "acme");
        assert_eq!(principal.subject, "svc-a");
        assert!(principal.operator);
        assert_eq!(principal.user_scope.as_deref(), Some("u1"));
    }

    #[test]
    fn invalid_credentials_are_refused() {
        let v = verifier();
        let cases = [
            ("wrong issuer", token(json!({"iss": "evil"}))),
            ("wrong audience", token(json!({"aud": "other"}))),
            ("expired", token(json!({"exp": NOW - 100}))),
            ("missing tenant", token(json!({"tenant": null}))),
            ("bad tenant charset", token(json!({"tenant": "a/../b"}))),
            ("uppercase tenant", token(json!({"tenant": "Acme"}))),
            ("empty tenant", token(json!({"tenant": ""}))),
            ("missing iat", token(json!({"iat": null}))),
            ("empty user_id", token(json!({"user_id": ""}))),
            ("empty subject", token(json!({"sub": " "}))),
            ("garbage", "not.a.jwt".to_string()),
        ];
        for (name, bad) in cases {
            assert!(v.verify_at(&bad, NOW).is_err(), "{name} must be refused");
        }
    }

    #[test]
    fn a_stale_or_future_token_fails_the_freshness_contract() {
        let v = verifier();
        let stale = token(json!({"iat": NOW - 301, "exp": NOW + 600}));
        assert!(v.verify_at(&stale, NOW).is_err());
        let future = token(json!({"iat": NOW + 60, "exp": NOW + 600}));
        assert!(v.verify_at(&future, NOW).is_err());
        let edge = token(json!({"iat": NOW - 300, "exp": NOW + 600}));
        assert!(v.verify_at(&edge, NOW).is_ok());
    }

    #[test]
    fn a_token_signed_with_another_key_is_refused() {
        let forged = encode(
            &Header::new(Algorithm::HS256),
            &json!({"iss":"gate","aud":"memory","sub":"x","tenant":"acme","iat":NOW,"exp":NOW+600}),
            &EncodingKey::from_secret(b"a-different-secret-of-32-bytes!!!"),
        )
        .unwrap();
        assert!(verifier().verify_at(&forged, NOW).is_err());
    }
}
