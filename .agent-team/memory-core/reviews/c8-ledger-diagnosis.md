# c8 diagnosis: ledger coordinator after concurrent query cancellations

- Role: memory-runtime (diagnosis only; no source edits, no cargo, no services, no DB access)
- Tree: `feat/surrealdb-3x-connection-model` @ `c7b2b34`
- SDK sources read: `surrealdb-3.3.0`, `surrealdb-3.2.4`, `surrealdb-engine-api-3.3.0`
  under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`
- Logs read: `~/.prometheus/logs/surreal-memory-native.stderr.log`,
  `~/.prometheus/logs/surrealdb-native.stderr.log` (ANSI-stripped copies were written to scratch only)
- Compass: the graph is stale for this question. `.compass/verification.json` sourceCommit
  `dd7fdcd` is not an ancestor of HEAD. `compass callers ledger_connection --engine json` returned
  `no_match`, and the default store engine fails with `snapshot format is unsupported`. Call sites
  below come from `rg` and from reading the source.
- Nothing here is executed evidence. The failing-test observations come from the lead's report.

## 1. From 4 cancelled receipt lookups to the later 503/500

### What the code does

- Every ledger method clones the root session of the current generation.
  - Sites: `src/operations.rs:584, 640, 656, 667, 689, 734, 786, 819, 861, 951, 972`
    (`let db = connection.db.clone();`).
  - SDK 3.3.0 `Clone` sends `SessionId::Clone{old,new}` (`surrealdb-3.3.0/src/lib.rs:337-350`).
  - `Drop` sends `SessionId::Drop` (`lib.rs:352-359`).
- In 3.3.0 a cloned session cannot dispatch anything until its replay has been acknowledged, one
  command at a time:
  - `handle_session_clone` copies the parent's replay log and starts the replay
    (`ws/mod.rs:939-963`).
  - `send_replay_command` sends one command and records its id as the cursor (`ws/mod.rs:856-900`).
  - `handle_route` parks every request while the cursor is set (`ws/mod.rs:230-237`).
  - Each acknowledgement sends the next command. Parked routes are flushed only after the last one
    (`ws/mod.rs:468-531`).
  - The replay log holds `Attach`, plus `Signin` and `Use` because both are replayable
    (`engine/remote/mod.rs:397-408`).
  - So one ledger call is Attach RTT, [Signin RTT when credentials exist], Use RTT, then the query
    RTT, all serialized through the single router task (`ws/native.rs:246-334`).
- In SDK 3.2.4 the same replay was pipelined fire-and-forget with `id: None`
  (`surrealdb-3.2.4/src/engine/remote/ws/mod.rs:586-610`). The query followed immediately with no
  wait for acknowledgements.

### Sequence in the test

1. The four GETs share generation 0.
   - `ledger_connection` opens it lazily under `connection_replacement` (`operations.rs:450-462`).
   - `SurrealStorage::operation_ledger_connection` opens it as an independent WebSocket
     (`crates/surreal-memory/src/storage/surreal.rs:944-951`).
   - Each GET clones a session, completes the replay, and sends the 32 MiB receipt SELECT.
2. At 10 ms each `tokio::time::timeout` drops its query future (`operations.rs:492`).
   - The SDK is not told. The pending entry stays in that session's `pending_requests`.
   - The late reply is then handled one of two ways:
     - Detached session: dropped silently (`ws/mod.rs:422-425`).
     - Session still attached: logged as `Failed to send query results to channel: SendError(..)`
       (`ws/mod.rs:540-546`). This happens because each GET keeps its `db` clone alive until
       `get()` returns, which includes the replacement window.
   - Nothing is poisoned. No `Connection reset` occurs, because the reset path only runs on socket
     errors (`ws/native.rs:309-320`, `ws/mod.rs:1028-1046`). The 32 MiB frame is below the 64 MiB
     default limit (`opt/websocket.rs:48-54`).
3. The first GET locks the mutex and publishes generation 1 (`operations.rs:464-480`).
   - The replacement budget is `max(10 ms, 1 s)` = 1 s (`operations.rs:495`).
   - The other GETs see the newer generation and return `recovered: true`.
   - All four return the expected 500 through `api_error` (`src/api/mod.rs:46-68`).
4. When the last GET returns, the last generation-0 handle drops.
   - `Inner` drops the route sender, so the generation-0 router sees `route_rx` closed, sends
     `Close` and exits (`ws/native.rs:263-270`).
   - The router is `biased` toward session events and routes (`ws/native.rs:255-262`), so it exits
     without decoding further frames.
5. The POST runs on generation 1. `submit` makes three ledger calls, each with its own 10 ms
   deadline:
   - receipt lookup (`operations.rs:531-535`);
   - the CREATE transaction (`operations.rs:580-600`);
   - a final receipt lookup (`operations.rs:618-623`).

   Each call is a fresh clone paying 3 serialized RTTs. Any miss becomes `SubmitError::Storage`
   and then 503 (`operations.rs:602-616`, `1604-1609`). In the run where the POST passed, the poll
   GET (3 RTTs) competed on the same router with the coordinator's clone-per-call traffic
   (`transition` alone makes 3 clones: `operations.rs:715-768`) and missed its 10 ms, giving the
   500 at test line 235.

### What is left broken

Nothing persistent was found in our code or in the SDK on this path:

- The generation advanced.
- The mutex is released on every path, including a cancelled replacement, because the guard drops
  with the future.
- The old router exits.
- No session is poisoned: `fail_replay` only runs on a rejected or unreadable replay ack
  (`ws/mod.rs:666-675`).

The failure is fresh deadline misses after recovery, surfaced without retry. The runtime makes
them deterministic:

- `#[tokio::test]` is current-thread (`tests/operation_query_deadline.rs:79`), unlike production
  `#[tokio::main]` (`src/main.rs:75`). The axum handlers, the coordinator, both SDK routers and all
  timers share one thread.
- During the replacement window the generation-0 router is still alive and may decode late 32 MiB
  frames. Test profile dependencies are unoptimized (`Cargo.toml:138-142`).
- Every later ledger call carries the same 10 ms budget.

## 2. Our code, our SDK usage, or an SDK change?

It is our usage, exposed by an SDK 3.3.0 behaviour change.

- **Our usage:** per-call `Surreal::clone()` on the ledger. It violates the rule this phase just
  adopted (`docs/lessons.md:17`, "never clone it per operation"). c1 removed per-call clones from
  `SurrealStorage::live_db` but did not touch `OperationService`.
- **The SDK change:** 3.3.0 turned the replay into an acknowledged, serialized handshake that parks
  the request (see the citations in section 1). 3.2.4 pipelined it.
- **Supporting history:**
  - The same test passed with the 3.2.4 lockfile
    (`openspec/changes/archive/2026-09-21-complete-operation-ledger-recovery/evidence.md:27-31`;
    `Cargo.lock` at `b844b8e` = 3.2.4, moved to 3.3.0 in `ad97c9c`).
  - The server CLI also moved to 3.3.0 at the same time, which is a confound (see section 5).
- **Two more usage facts:**
  - `connect_with_config` claims the SDK `query_timeout` bounds queries at the protocol layer
    (`surreal.rs:730-739`). In 3.3.0 that value is only consumed by the local and gRPC engines
    (`engine/local/mod.rs:331`, `engine/remote/grpc.rs:196`). The WebSocket path has no timeout
    handling at all.
  - So in server mode the outer `tokio::time::timeout` is the only deadline. A cancelled query keeps
    running on the server, up to the server's own 1-minute limit, which is visible in
    `surrealdb-native.stderr.log`.

## 3. Does this explain the live `connection replacement timed out` errors?

Only partly. The live family has a different trigger and the same amplifier.

- **Different trigger:**
  - The live binary (`~/.local/bin` and `/usr/local/bin`, mtime Sep 21 02:22; the deployment
    evidence names `bc3d1ea`) was built with the 3.2.4 lockfile, so the 3.3.0 serialized replay is
    not the cause there.
  - The errors start on 2026-09-21, the day this ledger deadline and replacement code shipped. The
    per-day "replacement timed out" counts are 98, 116, 25, 219, 465 and 362 from Sep 21 to Sep 26.
    Most of that predates the Sep 26 05:49 server move to 3.3.0.
  - The trigger is server and host latency. Direct receipt lookups took 4.4–19.6 s under 34.5 GB of
    swap (`openspec/changes/archive/2026-09-25-direct-operation-receipt-lookup/deployment-evidence.md:16-19`).
    The server log also shows 60 s node-registration timeouts and `Resource busy` conflicts.
  - `Failed to send query results` follows a `Connecting` line by 0.4–3 s, so the server answered
    each timed-out lookup 10.4–13 s after it was issued. Example: live log lines 58230–58240.
- **Same amplifier:**
  - Every deadline opens a brand-new WebSocket plus signin plus use, with the 1 s floor, here
    10 s (`operations.rs:495-499`). The mutex wait counts against each waiter's budget.
  - One slow connect therefore times out every queued waiter together: 19:05:49.995 and .999, two
    "receipt lookup" timeouts from a single `Connecting` at 19:05:39.36.
  - The cancelled half-open connect appears on the server as
    `Failed to upgrade WebSocket connection: operation was canceled` (server log 2026-09-25T10:23:31,
    22:11:16, 2026-09-26T01:16:43).
  - Per-call clones also replay `Signin` on every ledger call when credentials are configured
    (`engine/remote/mod.rs:397-408`). That is a server-side password verification per call, adding
    load exactly when the server is slow.
  - `record_executor_events` does a receipt lookup plus a write for every executor event
    (`operations.rs:924-962`), which multiplies calls during inference.
- **Result:** under sustained latency the generation never advances and each timeout adds server
  work. That is a convoy, not a recovery.

## 4. Proposed root-cause fix and gate

**Fix A, minimal, the one I recommend for c8.** Stop cloning the ledger session per call.

- Change: at the 11 sites listed in section 1, replace `let db = connection.db.clone();` with
  `let db = &connection.db;`. The `Arc<LedgerConnection>` already outlives each await. Queries
  borrow via `Cow::Borrowed` (`method/query.rs:31`).
- Files: `src/operations.rs` only. Inline test fixtures may need the same borrow if any clone
  `db` (check `operations.rs:1784-1800` and the test module).
- Interface effect: none public. No ledger SQL or schema change. HTTP status and error-text
  contracts are unchanged. Idempotency and replay semantics are unchanged.
- Session safety: ledger SQL uses bound variables and single-request `BEGIN…COMMIT` strings only
  (`operations.rs:590-593, 739-742, 843-845`). There is no `LET`, `USE` or `.set()` per call, so
  one session per generation is safe. c1 review Q1 reached the same conclusion for storage.
- Effect: one RTT per ledger call instead of 3 (4 with credentials). There is no attach, signin,
  use or detach churn per call, and no per-call password verification on the server.
- Trade-off: a cancelled query's late reply now reaches a live session and logs the SDK
  `Failed to send query results to channel`. This is cosmetic.

**Fix B, design decision for the lead; not part of the minimal change.** Stop replacing the
transport on a plain deadline.

- In 3.x a dropped query future does not damage the multiplexed socket (section 1, step 2).
- Replacement adds a connect and signin under exactly the overload that caused the timeout, and it
  convoys.
- Replace only on connection-class SDK errors, such as `Connection reset` or
  `The engine dropped the request without answering`
  (`surrealdb-engine-api-3.3.0/src/lib.rs:611-625`).
- This changes the archived spec `operation-ledger-connection-recovery` and the coordinator's
  retry-once rule (`operations.rs:360-377, 1010-1017, 1061-1072`). It needs storage review and
  product/transports sign-off, so keep it as a separate change.

**Rejected:**

- Raising the 10 ms deadline or adding retries to `submit`/`get`: this masks the problem.
- A session pool: that is the c5 decision.
- Switching the test to `multi_thread` alone: it matches production, but it hides the 3× RTT
  regression. It is acceptable only alongside Fix A, and only if the lead agrees.

**Gate for Fix A:**

1. `RUSTC_WRAPPER= cargo test --locked --test operation_query_deadline --no-default-features --features server-only`
   passes N/N serially. I suggest N = 20, lead to confirm. Both tests must pass, unchanged
   except for diagnostics.
2. `cargo test --locked --lib operations::tests::` and `--test executor_recovery` pass.
3. Receipt: an SDK trace or server log for one run shows the attach/signin count equals the
   number of ledger generations, not the number of ledger calls.
4. Add a body-printing assertion message at test lines 177, 221 and 235 so any future failure
   names its stage. Test files need a lead-assigned writer.
5. At the phase boundary, run the workspace quality gate once.

## 5. Uncertainty and what would settle it

- **The 503/500 bodies were not captured.** "Deadline miss" is inferred. The alternatives are
  another `check()` error, such as a transaction conflict, or a connector failure.
  - Settles it: the body in the assert message, or `RUST_LOG=surreal_memory_server=debug,surrealdb=trace`
    on one run.
- **The RTT cost is unmeasured.** No run has measured that 3 serialized RTTs exceed 10 ms here, and
  the 3.3.0 server CLI change is confounded with the client change.
  - Settles it: one run of the unchanged test with the 3.3.0 client against a 3.2.4 `surreal`
    binary, or with per-call timing; then the Fix A run.
- **The CPU contribution is unmeasured.** It is not known how much current-thread CPU contention
  (debug decode of late 32 MiB frames) contributes.
  - Settles it: one diagnostic run with `flavor = "multi_thread"` and no other change.
- **Credentials in the live config are assumed.** I did not verify that the live config sets them,
  which would make the per-call `Signin` replay real there.
  - Settles it: check the launch agent environment for credential variables without printing their
    values.
- **Server-side cancellation is unverified.** Whether a Detach or socket close cancels in-flight
  server queries was not checked in `surrealdb-core`/server source. It only affects how long
  abandoned 32 MiB work keeps the fixture server busy.
