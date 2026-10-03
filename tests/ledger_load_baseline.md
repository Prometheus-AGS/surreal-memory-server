# Ledger load baseline (#36)

Rows are appended by `tests/ledger_load.rs` (server mode, loopback
`surreal` fixture). Columns: ledger deadline, operations submitted, client
requests, client-visible ledger errors (rate), connection replacements,
failed or timed-out replacements, receipt GET p50/p99, wall time, operations
left uncommitted.

| run (UTC) | deadline | ops | requests | ledger errors | replaced | replacement failed | p50 | p99 | wall | uncommitted |
|---|---|---|---|---|---|---|---|---|---|---|
| main 5e63ecf+log · 2026-10-03T18:13Z | 5ms | 320 | 12015 | 421 (3.50%) | 43 | 0 | 1.44ms | 5.73ms | 10.1s | 0 |
| main 5e63ecf+log · 2026-10-03T18:14Z | 5ms | 320 | 9627 | 419 (4.35%) | 49 | 0 | 1.68ms | 5.84ms | 10.1s | 0 |
| main 5e63ecf+log · 2026-10-03T18:14Z | 5ms | 320 | 12746 | 539 (4.23%) | 54 | 0 | 1.45ms | 5.93ms | 10.1s | 0 |
| fix/36 · 2026-10-03T18:13Z | 5ms | 320 | 9524 | 498 (5.23%) | 59 | 0 | 1.66ms | 5.87ms | 10.1s | 0 |
| fix/36 · 2026-10-03T18:14Z | 5ms | 320 | 11077 | 415 (3.75%) | 57 | 0 | 1.43ms | 5.77ms | 10.1s | 0 |
| fix/36 · 2026-10-03T18:14Z | 5ms | 320 | 9509 | 483 (5.08%) | 54 | 0 | 1.73ms | 5.86ms | 10.1s | 0 |

## Findings (2026-10-03)

- At a 25 ms deadline no ledger query overran on loopback; 5 ms is the
  stress point used above. At 2 ms nothing commits on either build.
- A "probe before replacing" variant (replace only when a liveness query
  fails) was measured and **rejected**: 0 replacements, but 7–13% client
  errors and p99 11–15 ms; coalescing probes restored p99 (~6 ms) but raised
  errors to 17–29%. Every ledger query shares one WebSocket, and a fresh
  connection starts with an empty queue, so replacing helps here.
- The shipped changes are neutral on this harness. A loopback fixture cannot
  reproduce the production slow-host (swap) connects behind #36, so the root
  cause there still needs measurement on the affected host.
