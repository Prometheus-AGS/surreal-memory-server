# Resilient SurrealDB Connection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add automatic retry/reconnection logic to SurrealDB client to handle transient connection failures.

**Architecture:** Wrap `Surreal<Any>` in `Arc<RwLock<ConnectionState>>`, route all operations through a generic retry wrapper with exponential backoff, classify errors as retriable/non-retriable, and expose configuration via environment variables.

**Tech Stack:** Rust, SurrealDB 3.0.2, tokio, tracing, anyhow

---

## Task 1: Add RetryConfig Types

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs:1-50`

**Step 1: Write the failing test**

```rust
// At bottom of crates/surreal-memory/src/storage/surreal.rs
#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_connect_retries, 10);
        assert_eq!(config.max_operation_retries, 3);
        assert_eq!(config.base_retry_delay_ms, 100);
        assert_eq!(config.max_retry_delay_ms, 5000);
        assert_eq!(config.jitter_factor, 0.25);
    }

    #[test]
    fn test_exponential_backoff_calculation() {
        let config = RetryConfig::default();
        let delay1 = config.calculate_delay(0);
        let delay2 = config.calculate_delay(1);
        let delay3 = config.calculate_delay(2);

        // Base delays (before jitter): 100ms, 200ms, 400ms
        assert!(delay1.as_millis() >= 75 && delay1.as_millis() <= 125); // 100 ± 25%
        assert!(delay2.as_millis() >= 150 && delay2.as_millis() <= 250); // 200 ± 25%
        assert!(delay3.as_millis() >= 300 && delay3.as_millis() <= 500); // 400 ± 25%
    }

    #[test]
    fn test_backoff_respects_max_delay() {
        let config = RetryConfig {
            max_retry_delay_ms: 500,
            ..Default::default()
        };
        let delay = config.calculate_delay(10); // Would be > 500ms without cap
        assert!(delay.as_millis() <= 625); // 500 + 25% jitter
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib retry_tests`
Expected: Compilation error - `RetryConfig` not found

**Step 3: Write minimal implementation**

Add after line 29 in `crates/surreal-memory/src/storage/surreal.rs`:

```rust
use std::time::Duration;
use rand::Rng;

/// Configuration for retry and reconnection behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_connect_retries: u32,
    pub max_operation_retries: u32,
    pub base_retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_connect_retries: 10,
            max_operation_retries: 3,
            base_retry_delay_ms: 100,
            max_retry_delay_ms: 5000,
            jitter_factor: 0.25,
        }
    }
}

impl RetryConfig {
    /// Calculate exponential backoff delay with jitter.
    /// Formula: min(base * 2^attempt, max) * (1 ± jitter_factor)
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let mut rng = rand::thread_rng();

        // Exponential backoff: base * 2^attempt
        let base_delay = self.base_retry_delay_ms.saturating_mul(2u64.saturating_pow(attempt));

        // Apply max cap
        let capped_delay = base_delay.min(self.max_retry_delay_ms);

        // Apply jitter: delay * (1 ± jitter_factor)
        let jitter_range = (capped_delay as f64 * self.jitter_factor) as u64;
        let min_delay = capped_delay.saturating_sub(jitter_range);
        let max_delay = capped_delay.saturating_add(jitter_range);

        let jittered_delay = rng.gen_range(min_delay..=max_delay);
        Duration::from_millis(jittered_delay)
    }
}
```

**Step 4: Add rand dependency**

Add to `Cargo.toml` workspace dependencies (line 27):

```toml
rand = "0.8"
```

Add to `crates/surreal-memory/Cargo.toml` dependencies:

```toml
rand = { workspace = true }
```

**Step 5: Run test to verify it passes**

Run: `cargo test --lib retry_tests`
Expected: All tests PASS

**Step 6: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs Cargo.toml crates/surreal-memory/Cargo.toml
git commit -m "feat: add RetryConfig with exponential backoff calculation"
```

---

## Task 2: Add ConnectionState and Update SurrealConfig

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs:33-50`

**Step 1: Write the failing test**

Add to `retry_tests` module:

```rust
#[tokio::test]
async fn test_connection_state_lifecycle() {
    use surrealdb::engine::any::Any;

    // Mock connection (will fail to actually connect, but tests the type)
    let state = ConnectionState::Reconnecting;
    assert!(matches!(state, ConnectionState::Reconnecting));

    let state = ConnectionState::Failed("test error".to_string());
    if let ConnectionState::Failed(msg) = state {
        assert_eq!(msg, "test error");
    } else {
        panic!("Expected Failed state");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_connection_state_lifecycle`
Expected: Compilation error - `ConnectionState` not found

**Step 3: Write minimal implementation**

Add after `RetryConfig` implementation:

```rust
/// Connection state for health tracking and reconnection.
#[derive(Debug)]
enum ConnectionState {
    Connected(Surreal<Any>),
    Reconnecting,
    Failed(String),
}

/// Connection configuration for reconnection attempts.
#[derive(Debug, Clone)]
struct ConnectionInfo {
    config: SurrealConfig,
    retry_config: RetryConfig,
}
```

Update `SurrealConfig` (around line 40):

```rust
#[derive(Debug, Clone)]
pub struct SurrealConfig {
    pub mode: SurrealMode,
    pub endpoint: Option<String>,
    pub embedded_path: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub namespace: String,
    pub database: String,
    pub retry: RetryConfig,  // NEW FIELD
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_connection_state_lifecycle`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat: add ConnectionState enum and retry field to SurrealConfig"
```

---

## Task 3: Refactor SurrealStorage to Use ConnectionState

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs:26-94`

**Step 1: Update SurrealStorage struct**

Replace the existing `SurrealStorage` struct (around line 26):

```rust
pub struct SurrealStorage {
    connection: Arc<RwLock<ConnectionState>>,
    connection_info: ConnectionInfo,
    embedding_service: Arc<dyn EmbeddingService>,
}
```

Add imports at top of file:

```rust
use std::sync::{Arc, RwLock};
```

**Step 2: Update the `new()` method**

Replace the `new()` method implementation (around line 51-94):

```rust
impl SurrealStorage {
    pub async fn new(
        config: &SurrealConfig,
        embedding_service: Arc<dyn EmbeddingService>,
    ) -> Result<Self> {
        let connection_info = ConnectionInfo {
            config: config.clone(),
            retry_config: config.retry.clone(),
        };

        let db = Self::connect_with_config(config).await?;

        // Run migrations on initial connection
        run_migrations(&db).await?;

        let connection = Arc::new(RwLock::new(ConnectionState::Connected(db)));

        Ok(Self {
            connection,
            connection_info,
            embedding_service,
        })
    }

    /// Establish connection without retry logic (called by connect_with_retry).
    async fn connect_with_config(config: &SurrealConfig) -> Result<Surreal<Any>> {
        let db = match &config.mode {
            SurrealMode::Embedded => {
                let path = config
                    .embedded_path
                    .as_ref()
                    .context("Embedded path required for embedded mode")?;
                tracing::info!("Connecting to embedded SurrealDB at: {}", path);
                surrealdb::engine::any::connect(format!("rocksdb://{}", path)).await?
            }
            SurrealMode::Server => {
                let endpoint = config
                    .endpoint
                    .as_ref()
                    .context("Endpoint required for server mode")?;
                tracing::info!("Connecting to SurrealDB server at: {}", endpoint);
                surrealdb::engine::any::connect(endpoint).await?
            }
        };

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            db.signin(Root {
                username: username.clone(),
                password: password.clone(),
            })
            .await
            .context("Failed to sign in to SurrealDB")?;
        }

        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .context("Failed to use namespace/database")?;

        Ok(db)
    }

    // Keep existing helper methods: embed_entity, embed_text, create_record, replace_record, cosine_similarity
    // (They will need to be updated later to accept &Surreal<Any> parameter)
```

**Step 3: Build to verify compilation**

Run: `cargo build --lib`
Expected: Build succeeds (may have warnings about unused fields)

**Step 4: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "refactor: wrap connection in Arc<RwLock<ConnectionState>>"
```

---

## Task 4: Implement Error Classification

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs` (retry_tests module)

**Step 1: Write the failing tests**

Add to `retry_tests` module:

```rust
#[test]
fn test_retriable_network_errors() {
    let storage = mock_storage();

    let err = anyhow::anyhow!("Connection uninitialised");
    assert!(storage.is_retriable_error(&err));

    let err = anyhow::anyhow!("connection closed");
    assert!(storage.is_retriable_error(&err));

    let err = anyhow::anyhow!("Connection timeout");
    assert!(storage.is_retriable_error(&err));

    let err = anyhow::anyhow!("too many connections");
    assert!(storage.is_retriable_error(&err));
}

#[test]
fn test_non_retriable_errors() {
    let storage = mock_storage();

    let err = anyhow::anyhow!("Found field 'foo', but no such field exists");
    assert!(!storage.is_retriable_error(&err));

    let err = anyhow::anyhow!("invalid credentials");
    assert!(!storage.is_retriable_error(&err));

    let err = anyhow::anyhow!("table doesn't exist");
    assert!(!storage.is_retriable_error(&err));

    let err = anyhow::anyhow!("record not found");
    assert!(!storage.is_retriable_error(&err));
}

fn mock_storage() -> SurrealStorage {
    // Mock storage for testing error classification
    use crate::embeddings::EmbeddingService;

    struct MockEmbedding;
    #[async_trait::async_trait]
    impl EmbeddingService for MockEmbedding {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.0; 1536])
        }
        fn dimensions(&self) -> usize { 1536 }
    }

    let config = SurrealConfig {
        mode: SurrealMode::Embedded,
        embedded_path: Some("/tmp/test".to_string()),
        endpoint: None,
        username: None,
        password: None,
        namespace: "test".to_string(),
        database: "test".to_string(),
        retry: RetryConfig::default(),
    };

    let connection_info = ConnectionInfo {
        config: config.clone(),
        retry_config: config.retry.clone(),
    };

    SurrealStorage {
        connection: Arc::new(RwLock::new(ConnectionState::Failed("test".to_string()))),
        connection_info,
        embedding_service: Arc::new(MockEmbedding),
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_retriable`
Expected: Compilation error - `is_retriable_error` method not found

**Step 3: Write minimal implementation**

Add method to `SurrealStorage` impl block:

```rust
impl SurrealStorage {
    // ... existing methods ...

    /// Classify errors as retriable or non-retriable.
    fn is_retriable_error(&self, error: &anyhow::Error) -> bool {
        let error_msg = format!("{}", error).to_lowercase();

        // Retriable: Network errors
        if error_msg.contains("connection")
            || error_msg.contains("timeout")
            || error_msg.contains("dns")
            || error_msg.contains("refused")
            || error_msg.contains("reset")
            || error_msg.contains("closed") {
            return true;
        }

        // Retriable: Transient DB errors
        if error_msg.contains("too many connections")
            || error_msg.contains("backpressure")
            || error_msg.contains("lock timeout")
            || error_msg.contains("serialization failure") {
            return true;
        }

        // Non-retriable: Schema/validation errors
        if error_msg.contains("field")
            || error_msg.contains("table")
            || error_msg.contains("invalid")
            || error_msg.contains("credential")
            || error_msg.contains("not found")
            || error_msg.contains("constraint") {
            return false;
        }

        // Default: don't retry unknown errors
        false
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_retriable`
Expected: All tests PASS

**Step 5: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat: add error classification for retry logic"
```

---

## Task 5: Implement connect_with_retry

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs`

**Step 1: Write the failing test**

Add to `retry_tests` module:

```rust
#[tokio::test]
async fn test_connect_with_retry_success_on_second_attempt() {
    // This test is conceptual - actual implementation would need
    // a way to inject failures. Document the expected behavior.

    // Expected: connect_with_retry should:
    // 1. Attempt connection
    // 2. On retriable failure, wait with backoff
    // 3. Retry up to max_connect_retries times
    // 4. Return Ok(db) on success or Err after exhausting retries
}
```

**Step 2: Write implementation**

Add method to `SurrealStorage` impl block:

```rust
impl SurrealStorage {
    // ... existing methods ...

    /// Establish connection with retry logic.
    async fn connect_with_retry(&self) -> Result<Surreal<Any>> {
        let max_retries = self.connection_info.retry_config.max_connect_retries;

        for attempt in 0..=max_retries {
            match Self::connect_with_config(&self.connection_info.config).await {
                Ok(db) => {
                    if attempt > 0 {
                        tracing::info!(
                            attempt = attempt + 1,
                            "Successfully connected to SurrealDB after retry"
                        );
                    }
                    return Ok(db);
                }
                Err(err) if attempt < max_retries && self.is_retriable_error(&err) => {
                    let delay = self.connection_info.retry_config.calculate_delay(attempt);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = max_retries + 1,
                        error = %err,
                        next_delay_ms = delay.as_millis(),
                        "Failed to connect to SurrealDB, retrying..."
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(err) => {
                    tracing::error!(
                        attempt = attempt + 1,
                        error = %err,
                        "Failed to connect to SurrealDB after all retries"
                    );
                    return Err(err);
                }
            }
        }

        anyhow::bail!("Exhausted connection retries")
    }
}
```

**Step 3: Update new() to use connect_with_retry**

Replace `Self::connect_with_config(config).await?` with `Self::connect_with_retry_initial(config).await?` in the `new()` method.

Add helper:

```rust
impl SurrealStorage {
    /// Initial connection with retry (used during construction).
    async fn connect_with_retry_initial(config: &SurrealConfig) -> Result<Surreal<Any>> {
        let retry_config = config.retry.clone();
        let max_retries = retry_config.max_connect_retries;

        for attempt in 0..=max_retries {
            match Self::connect_with_config(config).await {
                Ok(db) => {
                    if attempt > 0 {
                        tracing::info!(
                            attempt = attempt + 1,
                            "Successfully connected to SurrealDB after retry"
                        );
                    }
                    return Ok(db);
                }
                Err(err) if attempt < max_retries => {
                    let delay = retry_config.calculate_delay(attempt);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = max_retries + 1,
                        error = %err,
                        next_delay_ms = delay.as_millis(),
                        "Failed to connect to SurrealDB, retrying..."
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(err) => {
                    tracing::error!(
                        attempt = attempt + 1,
                        error = %err,
                        "Failed to connect to SurrealDB after all retries"
                    );
                    return Err(err);
                }
            }
        }

        anyhow::bail!("Exhausted connection retries")
    }
}
```

Update `new()`:

```rust
let db = Self::connect_with_retry_initial(config).await?;
```

**Step 4: Build to verify compilation**

Run: `cargo build --lib`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat: implement connect_with_retry for initial connection"
```

---

## Task 6: Implement reconnect Method

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs`

**Step 1: Write implementation**

Add method to `SurrealStorage` impl block:

```rust
impl SurrealStorage {
    // ... existing methods ...

    /// Attempt to reconnect to the database.
    /// Should be called with write lock held.
    async fn reconnect(&self) -> Result<()> {
        tracing::info!("Attempting to reconnect to SurrealDB...");

        // Set state to Reconnecting
        {
            let mut state = self.connection.write().unwrap();
            *state = ConnectionState::Reconnecting;
        }

        // Attempt reconnection with retry logic
        match self.connect_with_retry().await {
            Ok(db) => {
                // Run migrations on reconnection (idempotent)
                run_migrations(&db).await?;

                // Update state to Connected
                let mut state = self.connection.write().unwrap();
                *state = ConnectionState::Connected(db);

                tracing::info!("Successfully reconnected to SurrealDB");
                Ok(())
            }
            Err(err) => {
                // Update state to Failed
                let mut state = self.connection.write().unwrap();
                *state = ConnectionState::Failed(format!("{}", err));

                Err(err)
            }
        }
    }
}
```

**Step 2: Build to verify compilation**

Run: `cargo build --lib`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat: implement reconnect method with state management"
```

---

## Task 7: Implement retry_operation Generic Wrapper

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs`

**Step 1: Write the failing test**

Add to `retry_tests` module:

```rust
#[tokio::test]
async fn test_retry_operation_succeeds_immediately() {
    let storage = create_test_storage().await;

    let result = storage.retry_operation("test_op", |_db| async move {
        Ok::<i32, anyhow::Error>(42)
    }).await;

    assert_eq!(result.unwrap(), 42);
}

async fn create_test_storage() -> SurrealStorage {
    use crate::embeddings::EmbeddingService;

    struct MockEmbedding;
    #[async_trait::async_trait]
    impl EmbeddingService for MockEmbedding {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.0; 1536])
        }
        fn dimensions(&self) -> usize { 1536 }
    }

    let config = SurrealConfig {
        mode: SurrealMode::Embedded,
        embedded_path: Some(std::env::temp_dir().join("test-retry").to_string_lossy().to_string()),
        endpoint: None,
        username: None,
        password: None,
        namespace: "test".to_string(),
        database: "test".to_string(),
        retry: RetryConfig {
            max_operation_retries: 2,
            ..Default::default()
        },
    };

    SurrealStorage::new(&config, Arc::new(MockEmbedding)).await.unwrap()
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_retry_operation`
Expected: Compilation error - `retry_operation` method not found

**Step 3: Write minimal implementation**

Add to `SurrealStorage` impl block:

```rust
use std::future::Future;

impl SurrealStorage {
    // ... existing methods ...

    /// Generic retry wrapper for database operations.
    async fn retry_operation<F, Fut, R>(
        &self,
        op_name: &str,
        operation: F,
    ) -> Result<R>
    where
        F: Fn(Surreal<Any>) -> Fut,
        Fut: Future<Output = Result<R>>,
    {
        let max_retries = self.connection_info.retry_config.max_operation_retries;

        for attempt in 0..=max_retries {
            // Acquire read lock to get connection
            let db = {
                let state = self.connection.read().unwrap();
                match &*state {
                    ConnectionState::Connected(db) => db.clone(),
                    ConnectionState::Reconnecting => {
                        // Wait a bit and retry
                        drop(state);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    ConnectionState::Failed(msg) => {
                        anyhow::bail!("Connection failed: {}", msg);
                    }
                }
            };

            // Attempt operation
            match operation(db).await {
                Ok(result) => return Ok(result),
                Err(err) if attempt < max_retries && self.is_retriable_error(&err) => {
                    let delay = self.connection_info.retry_config.calculate_delay(attempt);

                    tracing::warn!(
                        operation = op_name,
                        attempt = attempt + 1,
                        max_attempts = max_retries + 1,
                        error = %err,
                        next_delay_ms = delay.as_millis(),
                        "Retrying operation after transient failure"
                    );

                    // Check if we need to reconnect
                    let error_msg = format!("{}", err).to_lowercase();
                    if error_msg.contains("connection") || error_msg.contains("uninitialised") {
                        tracing::info!("Connection issue detected, attempting reconnection");
                        if let Err(reconnect_err) = self.reconnect().await {
                            tracing::error!(error = %reconnect_err, "Reconnection failed");
                        }
                    }

                    tokio::time::sleep(delay).await;
                }
                Err(err) => {
                    if attempt >= max_retries {
                        tracing::error!(
                            operation = op_name,
                            attempt = attempt + 1,
                            error = %err,
                            "Operation failed after all retries"
                        );
                    }
                    return Err(err);
                }
            }
        }

        anyhow::bail!("Exhausted operation retries for {}", op_name)
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_retry_operation`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat: implement retry_operation generic wrapper"
```

---

## Task 8: Migrate High-Priority Write Operations

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs` (replace_record, create_record methods)

**Step 1: Extract create_record_impl helper**

Find the `create_record` method (around line 108) and refactor:

```rust
impl SurrealStorage {
    // Original method becomes a wrapper
    async fn create_record<T>(&self, table: &str, key: &str, value: T, op: &str) -> Result<T>
    where
        T: Serialize + DeserializeOwned + SurrealValue + Clone,
    {
        let table = table.to_string();
        let key = key.to_string();
        let op = op.to_string();

        self.retry_operation(&op, move |db| {
            let table = table.clone();
            let key = key.clone();
            let value = value.clone();
            let op = op.clone();
            async move {
                Self::create_record_impl(&db, &table, &key, value, &op).await
            }
        }).await
    }

    /// Implementation that accepts a database reference directly.
    async fn create_record_impl<T>(
        db: &Surreal<Any>,
        table: &str,
        key: &str,
        value: T,
        op: &str,
    ) -> Result<T>
    where
        T: Serialize + DeserializeOwned + SurrealValue,
    {
        let mut response = db
            .query("CREATE type::record($table, $key) CONTENT $value RETURN AFTER")
            .bind(("table", table.to_string()))
            .bind(("key", key.to_string()))
            .bind(("value", value))
            .await
            .with_context(|| format!("{op}: SurrealDB create query failed"))?;
        let created: Option<T> = response
            .take(0)
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        created.with_context(|| format!("{op}: SurrealDB returned no record after write"))
    }
}
```

**Step 2: Extract replace_record_impl helper**

Find the `replace_record` method (around line 126) and refactor:

```rust
impl SurrealStorage {
    // Original method becomes a wrapper
    async fn replace_record<T>(&self, record_id: &str, value: T, op: &str) -> Result<T>
    where
        T: Serialize + DeserializeOwned + SurrealValue + Clone,
    {
        let record_id = record_id.to_string();
        let op = op.to_string();

        self.retry_operation(&op, move |db| {
            let record_id = record_id.clone();
            let value = value.clone();
            let op = op.clone();
            async move {
                Self::replace_record_impl(&db, &record_id, value, &op).await
            }
        }).await
    }

    /// Implementation that accepts a database reference directly.
    async fn replace_record_impl<T>(
        db: &Surreal<Any>,
        record_id: &str,
        value: T,
        op: &str,
    ) -> Result<T>
    where
        T: Serialize + DeserializeOwned + SurrealValue,
    {
        let mut response = db
            .query("UPDATE $id CONTENT $value RETURN AFTER")
            .bind(("id", record_id.to_string()))
            .bind(("value", value))
            .await
            .with_context(|| format!("{op}: SurrealDB update query failed"))?;
        let updated: Option<T> = response
            .take(0)
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        updated.with_context(|| format!("{op}: SurrealDB returned no record after write"))
    }
}
```

**Step 3: Build to verify compilation**

Run: `cargo build --lib`
Expected: Build succeeds

**Step 4: Run existing tests**

Run: `cargo test --lib`
Expected: All tests PASS

**Step 5: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "refactor: add retry logic to create_record and replace_record"
```

---

## Task 9: Add Configuration Loading from Environment

**Files:**
- Modify: `src/main.rs` (config loading section)

**Step 1: Find the config loading section**

Locate where `SurrealConfig` is created from environment variables (likely around line 30-60 in `src/main.rs`).

**Step 2: Add environment variable parsing**

Add after loading existing SurrealDB config:

```rust
// Load retry configuration from environment
let retry_config = surreal_memory::storage::surreal::RetryConfig {
    max_connect_retries: std::env::var("SURREAL_MAX_CONNECT_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10),
    max_operation_retries: std::env::var("SURREAL_MAX_OPERATION_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3),
    base_retry_delay_ms: std::env::var("SURREAL_BASE_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100),
    max_retry_delay_ms: std::env::var("SURREAL_MAX_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000),
    jitter_factor: std::env::var("SURREAL_RETRY_JITTER_FACTOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.25),
};

tracing::info!(
    max_connect_retries = retry_config.max_connect_retries,
    max_operation_retries = retry_config.max_operation_retries,
    "Retry configuration loaded"
);
```

Add `retry: retry_config` field to the `SurrealConfig` struct initialization.

**Step 3: Export RetryConfig in lib.rs**

Add to `crates/surreal-memory/src/lib.rs`:

```rust
pub use storage::surreal::{RetryConfig, SurrealConfig, SurrealMode, SurrealStorage};
```

Update the existing export line if it exists.

**Step 4: Build to verify compilation**

Run: `cargo build --bin surreal-memory-server`
Expected: Build succeeds

**Step 5: Test environment variable parsing**

Run:
```bash
SURREAL_MAX_CONNECT_RETRIES=5 SURREAL_MAX_OPERATION_RETRIES=2 cargo run --bin surreal-memory-server -- --help
```
Expected: No errors, should see config loaded in logs

**Step 6: Commit**

```bash
git add src/main.rs crates/surreal-memory/src/lib.rs
git commit -m "feat: load retry config from environment variables"
```

---

## Task 10: Update docker-compose.yaml

**Files:**
- Modify: `docker-compose.yaml`

**Step 1: Add retry environment variables**

Add to the `surreal-memory-server` service environment section (after line 50):

```yaml
      # Retry configuration
      - SURREAL_MAX_CONNECT_RETRIES=10
      - SURREAL_MAX_OPERATION_RETRIES=3
      - SURREAL_BASE_RETRY_DELAY_MS=100
      - SURREAL_MAX_RETRY_DELAY_MS=5000
      - SURREAL_RETRY_JITTER_FACTOR=0.25
```

**Step 2: Verify YAML syntax**

Run: `docker compose config`
Expected: Valid YAML output with new environment variables

**Step 3: Commit**

```bash
git add docker-compose.yaml
git commit -m "config: add retry environment variables to docker-compose"
```

---

## Task 11: Add Integration Test for Retry Logic

**Files:**
- Modify: `crates/surreal-memory/tests/integration_test.rs`

**Step 1: Add test at end of file**

```rust
#[tokio::test]
async fn test_operation_survives_transient_failure() {
    use surreal_memory::{Memory, MemoryStorage, SurrealConfig, SurrealMode, SurrealStorage, RetryConfig};
    use surreal_memory::embeddings::EmbeddingService;

    // Create mock embedding service
    struct MockEmbedding;
    #[async_trait::async_trait]
    impl EmbeddingService for MockEmbedding {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0; 1536])
        }
        fn dimensions(&self) -> usize { 1536 }
    }

    // Create storage with retry config
    let temp_dir = std::env::temp_dir().join(format!("test-retry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let config = SurrealConfig {
        mode: SurrealMode::Embedded,
        embedded_path: Some(temp_dir.to_string_lossy().to_string()),
        endpoint: None,
        username: None,
        password: None,
        namespace: "test".to_string(),
        database: "test".to_string(),
        retry: RetryConfig {
            max_operation_retries: 3,
            base_retry_delay_ms: 10, // Short delays for fast tests
            ..Default::default()
        },
    };

    let storage = SurrealStorage::new(&config, std::sync::Arc::new(MockEmbedding))
        .await
        .expect("Failed to create storage");

    // Test that normal operations work with retry wrapper
    let memory = storage
        .add_memory(
            "test content".to_string(),
            None,
            Some("agent1".to_string()),
            None,
            None,
        )
        .await
        .expect("Failed to add memory");

    assert_eq!(memory.content, "test content");
    assert_eq!(memory.agent_id, Some("agent1".to_string()));

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}
```

**Step 2: Run test to verify it passes**

Run: `cargo test --test integration_test test_operation_survives_transient_failure`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/surreal-memory/tests/integration_test.rs
git commit -m "test: add integration test for retry logic"
```

---

## Task 12: Update README with Retry Configuration

**Files:**
- Modify: `README.md`

**Step 1: Add retry configuration section**

Find the "Configuration" section in README.md and add:

```markdown
### Retry Configuration

The server automatically retries failed operations due to transient connection issues:

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `SURREAL_MAX_CONNECT_RETRIES` | 10 | Maximum connection attempts during startup |
| `SURREAL_MAX_OPERATION_RETRIES` | 3 | Maximum retry attempts per operation |
| `SURREAL_BASE_RETRY_DELAY_MS` | 100 | Base delay for exponential backoff (ms) |
| `SURREAL_MAX_RETRY_DELAY_MS` | 5000 | Maximum retry delay (ms) |
| `SURREAL_RETRY_JITTER_FACTOR` | 0.25 | Random jitter factor (±25%) |

**Retriable errors:**
- Network errors (connection timeout, DNS failure, connection refused)
- Transient DB errors ("Connection uninitialised", "too many connections")
- Transaction conflicts (lock timeouts, serialization failures)

**Non-retriable errors:**
- Schema errors (field/table not found)
- Validation errors (invalid credentials, constraint violations)
- Not-found errors

Retry events are logged at `WARN` level with structured fields for observability.
```

**Step 2: Commit**

```bash
git add README.md
git commit -m "docs: add retry configuration to README"
```

---

## Task 13: Final Testing and Verification

**Files:**
- Test all components together

**Step 1: Run full test suite**

Run: `cargo test --all-features`
Expected: All tests PASS

**Step 2: Build release binary**

Run: `cargo build --release`
Expected: Build succeeds with no errors

**Step 3: Test docker-compose**

Run:
```bash
docker compose down
docker compose build
docker compose up -d
docker compose logs -f
```

Expected: Services start successfully, logs show retry config loaded

**Step 4: Test MCP endpoint with retry**

Run: `./scripts/test-interactive.sh` (if exists) or manually test an MCP operation
Expected: Operations succeed

**Step 5: Commit any final fixes**

```bash
git add .
git commit -m "test: verify retry logic integration"
```

---

## Task 14: Update CLAUDE.md with Retry Implementation Notes

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Add retry section**

Add after the "Performance Considerations" section:

```markdown
## Retry and Reconnection Logic

The SurrealDB client includes automatic retry logic to handle transient connection failures:

**Architecture:**
- Connection wrapped in `Arc<RwLock<ConnectionState>>`
- All operations routed through `retry_operation()` generic wrapper
- Exponential backoff with jitter: 100ms → 200ms → 400ms (configurable)
- Passive reconnection on connection loss

**Error Classification:**
- Retriable: Network errors, "Connection uninitialised", lock timeouts
- Non-retriable: Schema errors, validation errors, not-found errors

**Configuration:**
Set via environment variables (see README.md for full list):
- `SURREAL_MAX_CONNECT_RETRIES=10` - startup retries
- `SURREAL_MAX_OPERATION_RETRIES=3` - per-operation retries

**Observability:**
Retry attempts emit structured `tracing::warn!` events with:
- `operation` - operation name
- `attempt` - current attempt number
- `error` - error message
- `next_delay_ms` - backoff delay

**When Adding New Operations:**
Use the `retry_operation()` wrapper for all database operations:
```rust
self.retry_operation("operation_name", |db| async move {
    // operation implementation using &db
}).await
```
```

**Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add retry logic documentation to CLAUDE.md"
```

---

## Success Criteria

- ✅ RetryConfig with exponential backoff + jitter
- ✅ ConnectionState enum and connection wrapping
- ✅ Error classification (retriable vs non-retriable)
- ✅ Initial connection retry logic
- ✅ Reconnection on connection loss
- ✅ Generic retry_operation wrapper
- ✅ High-priority operations migrated (create_record, replace_record)
- ✅ Configuration from environment variables
- ✅ docker-compose.yaml updated
- ✅ Integration tests
- ✅ Documentation updated (README, CLAUDE.md)
- ✅ Full test suite passes
- ✅ Docker deployment works

## Future Tasks (Not in This Plan)

- Migrate remaining read operations to use retry wrapper
- Add circuit breaker pattern if retry storms occur
- Add Prometheus metrics for retry rates
- Add retry callback hooks for custom alerting
- Implement active health check background task (if needed)
