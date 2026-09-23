//! Explicit authentication scope for shared SurrealDB servers.
use anyhow::{Context, Result};
use std::str::FromStr;
use surrealdb::{Surreal, engine::any::Any, opt::auth::{Database, Namespace, Root}};

use super::surreal::SurrealConfig;

/// Root remains the compatibility default for existing library consumers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SurrealAuthLevel {
    #[default]
    Root,
    Namespace,
    Database,
}

impl FromStr for SurrealAuthLevel {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "root" => Ok(Self::Root),
            "namespace" => Ok(Self::Namespace),
            "database" => Ok(Self::Database),
            _ => anyhow::bail!("SURREAL_AUTH_LEVEL must be root, namespace, or database"),
        }
    }
}

impl SurrealConfig {
    /// Used both by normal connections (including reconnect) and repair commands.
    pub async fn authenticate(&self, db: &Surreal<Any>) -> Result<()> {
        let (Some(username), Some(password)) = (&self.username, &self.password) else {
            return Ok(());
        };
        let username = username.clone();
        let password = password.clone();
        match self.auth_level {
            SurrealAuthLevel::Root => db.signin(Root { username, password }).await,
            SurrealAuthLevel::Namespace => db.signin(Namespace {
                namespace: self.namespace.clone(), username, password,
            }).await,
            SurrealAuthLevel::Database => db.signin(Database {
                namespace: self.namespace.clone(), database: self.database.clone(), username, password,
            }).await,
        }.context("Failed to sign in to SurrealDB at the configured authentication scope")?;
        Ok(())
    }
}
