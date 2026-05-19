# Tasks — eliminate-write-path-stalls

## 1. Bounded operation budget
- [x] 1.1 Add a single wall-clock deadline spanning all retry + reconnect attempts in `retry_operation` (`tokio::time::timeout` over `retry_operation_inner`)
- [x] 1.2 Return a typed error when the budget is exhausted (no open hang)
- [x] 1.3 Make the budget configurable — `RetryConfig.operation_deadline_ms`, env `SURREAL_OPERATION_DEADLINE_MS`, default 30000ms

## 2. De-amplify reconnect
- [x] 2.1 Prevent unbounded `connect_with_retry` (10×) from inside the per-operation retry loop — `reconnect_with_attempts(OPERATION_RECONNECT_ATTEMPTS=2)`
- [x] 2.2 Preserve the larger initial-startup connect budget — `connect_with_retry` still uses `max_connect_retries`; new `connect_with_attempts` parameterizes the cap
- [x] 2.3 Confirm legitimate transient-failure recovery still works (`test_retry_operation_succeeds_after_retry`, `test_connect_with_retry_succeeds_after_transient_failure` pass)

## 3. Observability
- [x] 3.1 Add a `tracing::debug_span!("retry_operation")` over the retry path

## 4. Verification
- [x] 4.1 Test (C-2): `test_create_task_stream_fast_fails_on_failed_connection` + `test_retry_operation_aborts_on_deadline` (real Connected DB, deadline path)
- [x] 4.2 `create_entity` shares the same `retry_operation`/`create_record` path — bounded by the same deadline
- [x] 4.3 Transient-failure recovery within budget still succeeds (existing retry tests pass)
- [x] 4.4 fmt + clippy + lib tests clean (56 tests pass; palace test build also unblocked)
- [x] 4.5 QA gate: rust-reviewer — first pass BLOCK (1 HIGH stranded Reconnecting state, 3 MEDIUM); refined (ReconnectGuard drop-guard, .instrument span, real deadline test, doc); re-review APPROVED

## 5. Cancellation safety (added during QA refinement)
- [x] 5.1 `ReconnectGuard` forces `Reconnecting → Failed` if reconnect is cancelled mid-connect (e.g. by the deadline)
- [x] 5.2 Regression test `test_reconnect_cancellation_does_not_strand_reconnecting`
