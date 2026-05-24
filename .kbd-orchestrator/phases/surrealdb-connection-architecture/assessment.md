# Assessment — surrealdb-connection-architecture

**Date**: 2026-05-24
**Phase goal** (as stated by user): Root-cause the architectural defects in how
`surreal-memory-server` manages its SurrealDB connection (remote `ws://` server
mode and embedded `rocksdb://` mode), and recommend an architecture grounded in
SurrealDB best practice — not a timeout-tweak workaround that breaks across
hardware profiles.

**Symptoms reported**:
1. Timeouts under load when connected to a remote SurrealDB instance
   (`docker-compose.yaml`: `ws://surrealdb:8000`).
2. Synchronization errors under load when running in embedded mode.
3. Behavior changes with CPU / machine resources — i.e. the failures are
   contention-driven, not deterministic.

---

## 1. Current architecture (as-built)

### 1.1 Connection topology

- **Mode selection** — `src/main.rs:219-239` reads `SURREAL_MODE` and assembles
  a `SurrealConfig` consumed by `SurrealStorage::new`.
- **Connect call** — `crates/surreal-memory/src/storage/surreal.rs:610` (embedded)
  and `:618` (server) both call `surrealdb::engine::any::connect(...)` exactly
  **once at startup**.
- **Storage handle** —
  [`SurrealStorage`](crates/surreal-memory/src/storage/surreal.rs:43-49):

  ```rust
  pub struct SurrealStorage {
      connection: Arc<std::sync::RwLock<ConnectionState>>,
      connection_info: ConnectionInfo,
      embedding_service: Arc<dyn EmbeddingService>,
      #[cfg(feature = "palace")]
      palace: tokio::sync::OnceCell<PalaceContext>,
  }
  ```

  The `ConnectionState` enum has variants `Connected(Surreal<Any>)`,
  `Reconnecting`, `Failed(String)`.

- **Doc/code conflict** — the same file's doc comment at lines 38-42 says:

  > `Surreal<Any>` is internally `Arc`-wrapped and `Clone`-safe; no additional
  > mutex or wrapper is needed for concurrent access.

  …yet the implementation wraps the handle in
  `Arc<std::sync::RwLock<ConnectionState>>`. The author *knew* the wrapper was
  unnecessary, then added it anyway to model the `Reconnecting` / `Failed`
  lifecycle. This is the architectural seam where the design diverged from
  SurrealDB's guidance.

### 1.2 Concurrency pattern at every hot path

Every storage method follows the same shape (47 occurrences of
`connection.read()` / `connection.write()` in `surreal.rs`):

```rust
let db = {
    let state = self.connection.read().expect("Connection lock poisoned");
    match &*state {
        ConnectionState::Connected(db) => db.clone(),
        ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
        ConnectionState::Failed(msg)   => anyhow::bail!("Connection failed: {}", msg),
    }
};
// ... use db ...
```
(reference: `surreal.rs:765-772`, `:783-790`, and ~45 other sites)

### 1.3 Retry / deadline wrapper

`retry_operation` (`surreal.rs:906`) wraps each call in a single wall-clock
`operation_deadline_ms` (default **30 s** — `surreal.rs:91`). Inside the
deadline it loops: extract handle → run operation → classify error → optionally
`reconnect_with_attempts(OPERATION_RECONNECT_ATTEMPTS=2)` → exponential backoff.

Error classification (`is_retriable_error`, `surreal.rs:814-850`) operates on
the **stringified** error message, looking for substrings like `"connection"`,
`"timeout"`, `"lock timeout"`, `"too many connections"`.

### 1.4 Retry knobs (env-driven, `src/main.rs:27-60`)

| Var | Default | Purpose |
|---|---|---|
| `SURREAL_MAX_CONNECT_RETRIES` | 10 | Startup connect attempts |
| `SURREAL_MAX_OPERATION_RETRIES` | 3 | Per-op retry budget |
| `SURREAL_BASE_RETRY_DELAY_MS` | 100 | Base for exp backoff |
| `SURREAL_MAX_RETRY_DELAY_MS` | 5000 | Cap |
| `SURREAL_RETRY_JITTER_FACTOR` | 0.25 | Jitter ± |

The user's complaint — *"tinkering with timeouts is bad architecture"* — is
correct: these knobs are the exposed surface of an architectural defect (see §2),
not a tuning interface.

### 1.5 SDK version

`Cargo.toml:13` pins `surrealdb = "3.0.5"`.

---

## 2. Architectural gap analysis (root causes)

These are ranked by likelihood of causing the reported symptoms. Findings #1
and #2 are the load-bearing ones; the rest fall out of them.

### Finding 1 — CRITICAL: blocking `std::sync::RwLock` held across an async control surface

`Arc<std::sync::RwLock<ConnectionState>>` is a **blocking** lock used inside an
async (`tokio`) runtime. It exhibits two specific failure modes that align
exactly with the user's symptoms:

a) **Writer starvation under load.** Every storage call grabs a *read* lock to
   clone the handle (47 sites). When a reconnect is needed,
   `reconnect_with_attempts` (`surreal.rs:862-897`) must grab the *write* lock
   to flip state to `Reconnecting`. A `std::sync::RwLock` does not guarantee
   writer fairness — under sustained read pressure (every MCP call holds a read
   guard), the write side can be starved. The reconnect then either stalls or
   loses the race, and surrounding operations time out via
   `operation_deadline_ms`.

b) **Lock poisoning ⇒ permanent process degradation.** Every guard call uses
   `.expect("Connection lock poisoned")`. If *any* `.await` inside a held read
   guard panics (e.g. an embedding panic, a serde panic, an OOM), the lock
   poisons and every subsequent operation panics on guard acquisition. There is
   no recovery path. This is a single point of failure for the whole process.

c) **Cross-await holding (latent).** Several read-guard scopes use a block
   expression to drop the guard before the `await`, which is correct — but the
   pattern is repeated 47 times by hand and is not enforced by a helper.
   Refactors that hold the guard across an `await` would silently introduce
   deadlocks and would not be caught by clippy without
   `await_holding_lock` lint enabled at deny level.

**SurrealDB's own guidance contradicts this design.** From the official
*"Tips and tricks on using the Rust SDK"* (surrealdb.com/blog, 2025) and the
*Multi-tenancy* docs:

> When you clone a `Surreal<C>` client instance, it creates a new session with
> independent state while sharing the underlying database connection. Sessions
> share the same physical connection for efficiency, are thread-safe and can be
> used concurrently across Tokio tasks…

> Clone the client so we can move it to a different Tokio task

`Surreal<Any>` is already an internally `Arc`-wrapped, clone-cheap, thread-safe
multiplexer over a single WebSocket (or embedded engine). **It does not need —
and is harmed by — an external lock.**

### Finding 2 — HIGH: no separation of the "live handle" concern from the "lifecycle state" concern

The codebase mixes two distinct concerns in one lock:
1. **Hot path** — every MCP call needs a `Surreal<Any>` handle. This should be
   contention-free (atomic-load fast path).
2. **Cold path** — connection lifecycle transitions (Connected/Reconnecting/
   Failed). This is rare and may take seconds.

By putting both behind the same `RwLock`, the cold path serializes the hot
path. Under load, the contention is on the lock itself, not on SurrealDB.

The correct shape is to make the hot path lock-free (e.g. `arc-swap::ArcSwap`
or `tokio::sync::watch`) and let the cold path swap a new `Arc<Surreal<Any>>`
atomically. The retry/reconnect state machine should be an *internal* concern
of a small connection-supervisor task, not a state observable by every MCP call.

### Finding 3 — HIGH: single physical WebSocket = head-of-line blocking for slow queries

In server mode, the entire process multiplexes through **one** `Surreal<Any>`
client that holds **one** WebSocket. The SDK does multiplex concurrent queries
over that socket — but:

- Hybrid search, mindmap updates, and `compress_memories` are documented as
  expensive (see `CLAUDE.md` performance section); a slow one stalls the socket's
  send/receive queue for everyone behind it.
- The 30 s `MINDMAP_UPDATE_TIMEOUT` (`surreal.rs:35`) and the 30 s
  `operation_deadline_ms` are *equal*, so a single slow mindmap write can
  consume the entire deadline budget for an unrelated `hybrid_search_memories`
  request waiting in line.

SurrealDB's own scaling guidance (surrealdb.com/blog/surrealdb-scalability) is
to **scale the compute layer horizontally** by running multiple stateless
SurrealDB nodes in front of a shared storage layer — *and* to let the
application open multiple client connections / sessions for workload isolation.

### Finding 4 — HIGH: embedded RocksDB mode is fundamentally single-process

`docker-compose.yaml:11-17` (server mode) uses `rocksdb:/data/database.db`
inside the SurrealDB container — fine. But the application embedded mode
(`SURREAL_MODE=embedded`, `surreal.rs:610`) opens the same RocksDB path
*inside* the application process.

Per RocksDB's documented locking model (PointLockManager — *16 lock stripes
per column family*), concurrent transactions on the same key range contend at
the storage engine layer. Under load:
- Many tokio tasks spawn many concurrent SurrealDB transactions.
- RocksDB stripe-locks serialize them.
- The application's own `RwLock` adds a second serialization layer on top.
- Failures surface as "lock timeout" / "serialization failure" strings — which
  *are* classified as retriable, so the retry loop hammers the same path again.

**The "synchronization errors under load" are an embedded-mode amplification
of the same blocking-lock problem from Finding 1.** The fix is not to widen
the retry budget. It is to (a) remove the application-level lock and (b) treat
embedded mode as **a single-process, single-writer mode for development and
small workloads only**, with concurrency-bounded by a semaphore rather than
contending at the RocksDB stripe-lock layer.

CLAUDE.md already notes (correctly):
> Embedded mode creates RocksDB files — Multiple processes CANNOT share the
> same embedded DB path simultaneously. For multi-process access, use server
> mode.

What is *not* documented is that even within a single process, embedded mode
has a much lower safe concurrency ceiling than server mode. The architecture
treats them as interchangeable; they are not.

### Finding 5 — MEDIUM: error classification by substring matching is brittle

`is_retriable_error` (`surreal.rs:814-850`) classifies errors by looking for
substrings like `"connection"`, `"timeout"`, `"field"`, `"constraint"` in
`format!("{}", error)`. This is:
- Locale-fragile (SDK error messages can change between point releases).
- Ambiguous: an error containing both `"field"` and `"timeout"` (e.g. a
  field-validation error during a slow query) is classified as **non-retriable**
  because `"field"` matches first. Correct, but only by accident.
- It conflates *transport* failures (which should reconnect) with *server-side
  busy* signals (which should backoff but **not** reconnect).

The SDK exposes a typed `surrealdb::Error` enum with variants for `Db`, `Api`,
etc. — discriminate on the variant, not the string.

### Finding 6 — MEDIUM: no per-query `Config::query_timeout` on the SDK client

The SurrealDB Rust SDK supports a per-client query timeout via `Config`:

```rust
let config = Config::default().query_timeout(Duration::from_millis(1500));
DB.connect::<Ws>(("127.0.0.1:8000", config)).await?;
```
(surrealdb.com/docs/languages/rust/methods/connect)

The current code uses `surrealdb::engine::any::connect(endpoint)` without a
`Config`, so the SDK applies no query timeout. The only deadline is the
application-side `operation_deadline_ms` *after* the call has been issued.
A query that hangs on the server side runs until the application deadline,
holds its slot on the multiplexed socket for that whole time, and head-of-line
blocks unrelated work (per Finding 3).

### Finding 7 — MEDIUM: keep-alive / heartbeat lives outside the SDK

`fix(mcp): extend Streamable HTTP session keep-alive to 24h` (commit `0a7164a`)
keeps the *MCP* session alive but does nothing for the *SurrealDB* WebSocket.
If a NAT / load balancer / firewall in front of SurrealDB idles a quiet
connection, the first request after the idle period fails, triggers a
reconnect (which fights the lock — Finding 1), and the user sees a timeout.

### Finding 8 — LOW: `palace` adapter takes `connection_arc()` and replays the same lock pattern

`surreal.rs:806-809`:
```rust
pub(crate) fn connection_arc(&self) -> Arc<std::sync::RwLock<ConnectionState>> {
    Arc::clone(&self.connection)
}
```
…hands the same lock to `PalaceAdapter`. So Finding 1 multiplies: palace
queries contend on the same lock as memory queries. When palace runs its
3-way concurrent hybrid search (per CLAUDE.md), it grabs the read guard three
times concurrently with whatever else is in flight.

---

## 3. What "best practice" actually looks like

Cross-referenced from official SurrealDB documentation and SDK source:

| Concern | Best-practice shape | What this codebase does |
|---|---|---|
| Hold the client | `static DB: LazyLock<Surreal<Client>> = …` or `Arc<Surreal<Any>>` — clone freely, share across tasks | Wraps in `Arc<std::sync::RwLock<…>>` ❌ |
| Concurrency | Clone the handle per task; SDK multiplexes over one WS | Same — but blocks on a lock to clone ❌ |
| Per-query timeout | `Config::default().query_timeout(...)` at connect | Not used ❌ |
| Workload isolation | Open additional client connections (`Surreal::clone()`) for hot vs. background workloads | Single shared client ❌ |
| Horizontal scale (server mode) | Multiple stateless SurrealDB nodes behind LB, point app at the LB; or run multiple app instances at one node | Single node, single client ❌ |
| Reconnection | SDK auto-reconnects on WebSocket loss — listen for `Disconnected` event if you need to react | Reimplements a reconnect state machine externally ❌ |
| Connection lifecycle | Atomic swap of the active handle (`ArcSwap`, `tokio::sync::watch`) | Blocking `RwLock` ❌ |
| Embedded mode | Single-process, dev/small-workload only; bound concurrency with a semaphore | Treated as a peer of server mode ❌ |

---

## 4. Sycophancy-correction pass

The user asked specifically for this. Self-review of the analysis above:

| Claim | Is it grounded? |
|---|---|
| `std::sync::RwLock` is the wrong primitive here | **Yes** — directly contradicted by the file's own doc comment at `surreal.rs:38-42` and by SurrealDB's published Rust SDK guidance. The author identified this in writing and built around it anyway. |
| Lock poisoning is a real risk | **Yes** — `.expect("Connection lock poisoned")` appears 47× with no recovery path. A single panic anywhere taints the whole process. |
| Writer starvation explains the timeouts | **Plausible, not proven**. To confirm we need a load test that traces lock-acquire latency. Listed as the *most likely* cause given the symptom pattern (load-dependent, hardware-sensitive), but the assessment should not assert it as fact without a repro. |
| Embedded mode "synchronization errors" come from layered locking | **Plausible**. RocksDB's stripe-lock model is documented; the application-level lock is documented; the interaction is the obvious next layer. Confirming requires running the embedded mode under load with the application `RwLock` removed and observing whether the errors persist. |
| The retry knobs are "bad architecture" | **Agreed with caveats**. They are not *useless* — they let an operator stop the bleeding. But they are not a substitute for fixing the lock topology. The user's framing is correct: tuning them only papers over the defect. |
| "Just remove the lock" is the whole fix | **No, and the assessment should not say so**. Removing the lock fixes Finding 1, but Findings 3 (HOL blocking on a single WS), 6 (no `query_timeout`), and 4 (embedded concurrency ceiling) remain. The right fix is a small connection-supervisor design — see §5. |

Net: the assessment names a *likely* root cause with high confidence, flags
where confirmation is needed, and resists the temptation to over-attribute
every symptom to a single defect.

---

## 5. Recommended architecture (to be plan-validated, not adopted blindly)

A planning-level sketch — not a prescription. The Plan phase should pressure-test
each piece.

1. **Replace `Arc<std::sync::RwLock<ConnectionState>>` with `Arc<ArcSwap<ConnectionHandle>>`**
   (or `tokio::sync::watch<Arc<Surreal<Any>>>`). Hot path becomes an atomic
   load; reconnect becomes an atomic swap. No lock contention, no poisoning.

2. **Introduce a `ConnectionSupervisor` task** — owns the lifecycle
   (Connected/Reconnecting/Failed), listens for SDK disconnect events, and
   publishes the live handle via the atomic cell. Storage methods only ever
   *read* the current handle.

3. **Pass a `surrealdb::opt::Config::query_timeout(...)` into `connect`** so
   the SDK enforces a per-query deadline at the protocol layer. Application
   `operation_deadline_ms` becomes a *backstop*, not the primary mechanism.

4. **Discriminate retries on typed `surrealdb::Error` variants**, not strings.
   Separate "transport lost — reconnect" from "server busy — backoff" from
   "server returned a structural error — fail immediately".

5. **For server mode under load**: open `N` cloned client sessions (`Surreal::clone()`)
   and round-robin write-heavy vs. read-heavy work across them, so a slow write
   does not head-of-line-block a fast search. SurrealDB's multi-tenancy doc
   confirms cloned sessions share the connection efficiently.

6. **For embedded mode**: explicitly cap in-flight ops with a
   `tokio::sync::Semaphore` sized to the underlying RocksDB stripe count
   (default 16). Document embedded mode as "dev / single-tenant"; do not
   present it as a horizontal-scale path.

7. **Add an SDK-level keep-alive ping** on a watchdog timer when in server mode
   (every 15-30 s), so idle WebSocket drops are detected proactively and
   reconnected before a user request hits the dead socket.

8. **Add structured load tests** under `crates/surreal-memory/tests/` that
   reproduce the load symptoms (N concurrent `hybrid_search_memories` against
   both server and embedded modes). Without a repro, fixes are unfalsifiable.

---

## 6. Open questions for the user before planning

1. **Target deployment shape** — is the goal to scale the single-process
   `surreal-memory-server` vertically (one binary, many concurrent requests),
   or to run multiple instances behind a load balancer in front of a shared
   server-mode SurrealDB? The right concurrency primitives differ.

2. **Is embedded mode still a supported production target**, or only a
   dev/local convenience? If the latter, the "single semaphore + clear
   warning" recommendation in §5.6 is sufficient and we can stop pretending it
   has a horizontal-scale story.

3. **Acceptable disruption budget** — `MemoryStorage` is consumed by UAR (per
   `CLAUDE.md`). The connection-handle refactor is internal to `SurrealStorage`
   and should not change the trait, but a wider change to introduce typed
   errors *would* be a trait-surface change that ripples into UAR.

4. **Are there observed log snippets** of the actual timeouts/sync errors?
   Confirming the failure signatures match Finding 1/4 (vs. some other cause
   like embedding-model contention or HNSW index contention) is cheap and
   high-value before planning.

---

## 7. Phase artifacts

- This file: `.kbd-orchestrator/phases/surrealdb-connection-architecture/assessment.md`
- `progress.json` to be created by `/kbd-plan` next.

## 8. Next recommended step

Run `/kbd-plan surrealdb-connection-architecture` after the user answers the
open questions in §6. The plan should sequence: load-repro harness → atomic
swap refactor → typed-error retries → keep-alive → embedded semaphore →
documentation update.
