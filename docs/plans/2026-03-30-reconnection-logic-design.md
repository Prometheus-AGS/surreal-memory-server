# Resilient SurrealDB Connection with Retry Logic

**Date:** 2026-03-30
**Status:** Approved
**Author:** Claude (with user validation)

## Problem Statement

The surreal-memory-server encountered a production failure: `add_mindmap_node: SurrealDB rejected the write -> Connection uninitialised`. This was a transient WebSocket connection failure that caused the operation to fail immediately with no retry. The current implementation has no retry/reconnection logic, making the system fragile to network hiccups and database restarts.

**Requirements:**
- Initial connection retries during startup (configurable, default: 10 attempts)
- Per-operation retries for transient failures (configurable, default: 3 attempts)
- Passive reconnection on connection loss + optional event subscription (hybrid approach)
- Comprehensive error classification (network, transient DB, serialization conflicts)
- Structured observability (tracing events for debugging)
- Backward compatibility with UAR and existing deployments

## Architecture Overview

### Core Changes

Refactor `SurrealStorage` to wrap the database connection in `Arc<RwLock<ConnectionState>>`:

```rust
enum ConnectionState {
    Connected(Surreal<Any>),
    Reconnecting,
    Failed(String),
}

pub struct SurrealStorage {
    connection: Arc<RwLock<ConnectionState>>,
    connection_info: ConnectionInfo,
    embedding_service: Arc<dyn EmbeddingService>,
}
```

All database operations route through a generic `retry_with_reconnect()` method that:
1. Acquires read lock on connection state
2. If `Connected`, attempts the operation
3. On retriable failure, acquires write lock and attempts reconnection
4. Retries operation with exponential backoff + jitter
5. Logs structured retry events via `tracing::warn!`

### Configuration

New fields in `SurrealConfig`:

```rust
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_connect_retries: u32,      // default: 10
    pub max_operation_retries: u32,    // default: 3
    pub base_retry_delay_ms: u64,      // default: 100
    pub max_retry_delay_ms: u64,       // default: 5000
    pub jitter_factor: f64,            // default: 0.25 (±25%)
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
```

Environment variables:
```bash
SURREAL_MAX_CONNECT_RETRIES=10       # Initial connection attempts
SURREAL_MAX_OPERATION_RETRIES=3      # Per-operation retry attempts
SURREAL_BASE_RETRY_DELAY_MS=100      # Starting backoff delay
SURREAL_MAX_RETRY_DELAY_MS=5000      # Maximum backoff delay
SURREAL_RETRY_JITTER_FACTOR=0.25     # Jitter as decimal (0.25 = ±25%)
```

## Component Details

### New Types

```rust
pub struct ConnectionInfo {
    config: SurrealConfig,
    retry_config: RetryConfig,
}
```

### Key Methods

- `async fn connect_with_retry(&self) -> Result<Surreal<Any>>` - establishes initial connection with retry logic
- `async fn reconnect(&self) -> Result<()>` - attempts reconnection, updates `ConnectionState`
- `async fn retry_operation<F, R, Fut>(&self, op_name: &str, op: F) -> Result<R>` where `F: Fn(Surreal<Any>) -> Fut` - generic retry wrapper
- `fn is_retriable_error(&self, error: &anyhow::Error) -> bool` - classifies errors as retriable

## Error Classification & Retry Logic

### Retriable Errors

The system will automatically retry these error categories:

1. **Network errors**: Connection timeout, DNS failure, connection refused, connection reset
2. **Transient DB errors**: "Connection uninitialised", "too many connections", "connection closed"
3. **Transaction conflicts**: Lock timeouts, serialization failures
4. **Resource exhaustion**: Temporary out-of-memory, backpressure signals

### Non-Retriable Errors

These fail immediately without retry:
- Schema errors ("field not found", "table doesn't exist")
- Validation errors ("invalid record ID", "constraint violation")
- Authentication failures ("invalid credentials")
- Not-found errors (404 equivalent)

### Retry Flow

```
Operation attempt
  ↓
Error occurred? → NO → Return success
  ↓ YES
  ↓
Retriable? → NO → Return error immediately
  ↓ YES
  ↓
Attempts exhausted? → YES → Return final error
  ↓ NO
  ↓
Log retry event (tracing::warn!)
  ↓
Connection dead? → YES → Reconnect with write lock
  ↓ NO
  ↓
Sleep with exponential backoff + jitter
  ↓
Retry operation (loop back to top)
```

### Structured Logging

Each retry emits:

```rust
tracing::warn!(
    operation = op_name,
    attempt = attempt_num,
    max_attempts = max_retries,
    error = %err,
    next_delay_ms = delay.as_millis(),
    "Retrying operation after transient failure"
);
```

## Data Flow & Operation Wrapping

### Current Pattern

```rust
async fn add_mindmap_node(&self, ...) -> Result<MindMap> {
    let mut mm = self.get_mindmap(...).await?;
    mm.nodes.push(node);
    self.replace_record(&record_key, mm, "add_mindmap_node").await
}
```

### New Pattern with Retry Wrapper

```rust
async fn add_mindmap_node(&self, ...) -> Result<MindMap> {
    self.retry_operation("add_mindmap_node", |db| async move {
        let mut mm = Self::get_mindmap_impl(&db, ...).await?;
        mm.nodes.push(node);
        Self::replace_record_impl(&db, &record_key, mm, "add_mindmap_node").await
    }).await
}
```

### Migration Strategy

1. **Phase 1**: Add `RetryConfig` to `SurrealConfig`, implement retry infrastructure
2. **Phase 2**: Extract core logic into `_impl` helper methods accepting `&Surreal<Any>`
3. **Phase 3**: Migrate high-priority operations (writes, queries) to use retry wrapper
4. **Phase 4**: Migrate remaining operations (reads, searches)
5. **Phase 5**: Remove old direct `self.db` access patterns

This allows incremental rollout without breaking existing callers.

## Configuration & Environment Integration

### docker-compose.yaml Updates

```yaml
services:
  surreal-memory-server:
    environment:
      # Existing...
      - SURREAL_MODE=server
      - SURREAL_ENDPOINT=ws://surrealdb:8000

      # New retry settings (with defaults shown)
      - SURREAL_MAX_CONNECT_RETRIES=10
      - SURREAL_MAX_OPERATION_RETRIES=3
      - SURREAL_BASE_RETRY_DELAY_MS=100
      - SURREAL_MAX_RETRY_DELAY_MS=5000
```

### Configuration Precedence

1. Environment variables (highest priority)
2. `.env` file
3. Defaults from `RetryConfig::default()`

### Backward Compatibility

All retry fields have defaults, so existing deployments without these variables will use sensible defaults (10 connect retries, 3 operation retries). UAR can upgrade without code changes.

## Testing Strategy

### Unit Tests (in `crates/surreal-memory/src/storage/surreal.rs`)

1. **Retry logic tests:**
   - `test_exponential_backoff_calculation()` - verify delay progression with jitter
   - `test_error_classification()` - verify retriable vs non-retriable errors
   - `test_max_retries_exhaustion()` - ensure it fails after max attempts

2. **Connection state tests:**
   - `test_reconnect_on_connection_failure()` - simulate connection drop, verify reconnection
   - `test_concurrent_operations_during_reconnect()` - verify RwLock behavior with multiple readers

### Integration Tests (in `crates/surreal-memory/tests/integration_test.rs`)

3. **Real database tests:**
   - `test_initial_connection_retry()` - start with DB down, bring it up after 2 retries, verify success
   - `test_operation_retry_on_transient_failure()` - simulate network hiccup mid-operation
   - `test_embedded_mode_retry()` - verify retry logic works with RocksDB (no network)

### Testing Tools

- Use `tokio::time::pause()` for time-based tests (fast, deterministic)
- Mock transient errors by creating a wrapper that injects failures
- For network tests, use `docker-compose` with `toxiproxy` to simulate connection failures
- Verify structured logging output (capture tracing events in tests)

## Edge Cases & Considerations

### Edge Cases Handled

1. **Concurrent reconnection attempts** - RwLock write ensures only one thread reconnects at a time
2. **Reconnection during active operations** - Read lock holders finish their operation, new operations wait for write lock to complete
3. **Embedded mode** - Reconnection attempts still use retry logic but will typically fail immediately (no network latency)
4. **Migration running during reconnection** - Migrations only run once at initial `new()`, not on reconnect
5. **Poison lock scenarios** - If reconnection panics, RwLock is poisoned but we can recover by unwrapping the guard and attempting reconnection again

### Rollout Strategy

**Phase 1 (Week 1):** Infrastructure only
- Add `RetryConfig`, `ConnectionState`, and retry infrastructure
- No operation wrapping yet - just scaffolding
- Deploy to development environment

**Phase 2 (Week 2):** High-priority operations
- Wrap write operations (create/update/delete) with retry logic
- Monitor structured logs for retry events
- Deploy to staging

**Phase 3 (Week 3):** Remaining operations
- Wrap read operations and searches
- Full production rollout with monitoring

### Monitoring & Observability

- **Track retry rate** via log aggregation (search for "Retrying operation")
- **Alert on**:
  - Retry rate > 10% of operations
  - Connection state = Failed for > 1 minute
- **Dashboard metrics**:
  - Successful retries
  - Failed retries
  - Average retry delay
  - Connection state distribution

## Success Criteria

1. ✅ Initial connection retries handle DB startup delays
2. ✅ Operation-level retries recover from transient "Connection uninitialised" errors
3. ✅ Passive reconnection detects and recovers from connection drops
4. ✅ Structured logging provides visibility into retry behavior
5. ✅ Configuration via environment variables works in docker-compose
6. ✅ Backward compatible - UAR upgrades without code changes
7. ✅ Test coverage for retry logic, error classification, and concurrent access

## Non-Goals

- Active health check background tasks (passive detection is sufficient)
- Connection pooling (not needed for embedded mode, overkill for server mode)
- Circuit breaker pattern (may add in future if needed)
- Retry budget / rate limiting (not required for single-tenant deployment)

## Future Enhancements

- Add circuit breaker if retry storms occur
- Expose retry metrics via Prometheus endpoint
- Add retry callback hooks for custom alerting
- Support for custom error classifiers per deployment
