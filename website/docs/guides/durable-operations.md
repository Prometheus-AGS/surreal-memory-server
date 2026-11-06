---
id: durable-operations
title: Durable operations
sidebar_position: 3
---

# Durable operations

`POST /api/v2/operations` is the write path for integrations that cannot afford to
lose or duplicate a write. You choose the operation ID and the server records a
**receipt** before it does any work, so after a timeout, a crash or a restart you
ask for the receipt instead of guessing.

A receipt moves through these states, in this order, with an event recorded for
each step:

`accepted` → `validated` → `planned` → `processing` → `indexed` → `committed`

An operation whose dependencies have not committed waits in `blocked`, and one that
can never succeed ends in `rejected`.

Only `committed` and `rejected` are terminal. Everything else is in progress, and
a paused operation is retried automatically every 60 seconds.

## Submit an operation

The body has six fields:

| Field | Meaning |
|---|---|
| `operation_id` | your ID, unique within the tenant |
| `schema_version` | `2` |
| `kind` | one of the kinds below |
| `dependencies` | operation IDs that must be `committed` first (may be empty) |
| `payload_hash` | SHA-256 of the canonical payload (next section) |
| `payload` | the kind-specific body |

### The canonical payload hash

`payload_hash` is the lowercase hex SHA-256 of the payload serialized as **compact
JSON with object keys sorted**, with no spaces and UTF-8 text left as is. The server
recomputes it and rejects a mismatch with `400`, naming the hash it expected.

```bash
payload='{"content":"Bob deploys on Fridays","user_id":"bob","agent_id":"claude-code"}'

hash=$(printf '%s' "$payload" | python3 -c '
import sys, json, hashlib
p = json.load(sys.stdin)
canonical = json.dumps(p, separators=(",", ":"), sort_keys=True, ensure_ascii=False)
print(hashlib.sha256(canonical.encode()).hexdigest())')

curl -s -X POST http://127.0.0.1:3001/api/v2/operations \
  -H 'content-type: application/json' \
  -d "{\"operation_id\":\"op-001\",\"schema_version\":2,\"kind\":\"add_memory\",
       \"dependencies\":[],\"payload_hash\":\"$hash\",\"payload\":$payload}"
```

:::tip Avoid floating-point numbers in payloads
Integers, strings, booleans, arrays and objects canonicalize identically in every
language. Floats can serialize differently between your client and the server.
:::

### Responses

| Status | Meaning |
|---|---|
| `202 Accepted` | a new operation is durably accepted |
| `200 OK` | exact replay: same ID **and** same hash. Safe to retry blindly |
| `409 Conflict` | that ID already exists with a **different** payload hash |
| `400 Bad Request` | invalid payload, unknown kind, or hash mismatch |
| `503` | the ledger is temporarily unavailable; the operation was not accepted |

A conflict says which hash is on record:

```json
{"error":"operation_id 'op-001' already exists with payload_hash eb9effd3…c018"}
```

The `202` body is the receipt:

```json
{
  "operation_id": "op-001",
  "schema_version": 2,
  "kind": "add_memory",
  "payload_hash": "eb9effd314920ae4ffcfde8ff895b25220eb4b9997fae428b9acc6348c97c018",
  "dependencies": [],
  "state": "accepted",
  "blocked_by": [],
  "result": null,
  "error": null,
  "progress_seq": 1,
  "created_at": "2026-11-06T00:02:07.078216Z"
}
```

## Kinds and payloads

| Kind | Payload | Does |
|---|---|---|
| `add_memory` | `content`, optional `user_id`, `agent_id`, `session_id`, `categories` | stores one logical memory, however long, in embedded parts |
| `create_task_stream` | `name`, optional `description`, `agent_id`, `user_id` | creates a TaskStream |
| `add_task_step` | `stream_name`, `name`, `idempotency_key`, optional `ordinal` (default 1), `description`, `agent_id`, `user_id` | appends a step |
| `complete_step` | `idempotency_key`, optional `result` | completes the step with that key |
| `rekey_agent_id` | `to_agent_id`, optional `user_id`, `dry_run` | rewrites agent ownership (operator only) |

Long memories are split by the active tokenizer. The plan and the executor's
progress are persisted, so after a restart the same plan is reused and only the
unfinished parts are embedded before one logical memory is committed.

## Dependencies

List operation IDs a step needs. The operation waits in `blocked` and `blocked_by`
shows what it is waiting for:

```json
{ "operation_id": "step-1", "kind": "add_task_step",
  "dependencies": ["stream-1"], "...": "..." }
```

Dependencies are looked up **within your tenant**, so naming another tenant's
operation ID never unblocks anything.

## Read the receipt

```bash
curl -s http://127.0.0.1:3001/api/v2/operations/op-001
```

```json
{ "operation_id": "op-001", "kind": "add_memory", "state": "committed",
  "progress_seq": 6, "blocked_by": [], "error": null,
  "result": { "id": { "table": "memory", "key": { "String": "…" } },
              "content": "Bob deploys on Fridays", "version": 1, "…": "…" } }
```

`result` holds what the operation produced. For `add_memory` it is the stored
memory.

## Follow events

Events are a server-sent-event stream, ordered by `sequence`. Pass `after` to resume
from the last one you processed:

```bash
curl -N "http://127.0.0.1:3001/api/v2/operations/op-001/events?after=0"
```

```text
event: operation_state
id: 1
data: {"operation_id":"op-001","sequence":1,"from_state":null,"to_state":"accepted",
       "detail":{"receipt":"durably accepted"},"executor_generation":0,
       "occurred_at":"2026-11-06T00:02:07.078216Z"}

event: operation_state
id: 2
data: {"operation_id":"op-001","sequence":2,"from_state":"accepted",
       "to_state":"validated","detail":{"contract":"validated"}, "…": "…"}
```

## Operate on stuck work

```bash
# Counts per state, and the oldest unfinished operation. Alert on age_seconds.
curl -s http://127.0.0.1:3001/api/v2/operations/stats
```

```json
{
  "counts": { "committed": 1 },
  "nonterminal": 0,
  "paused": 0,
  "oldest_nonterminal": null,
  "query_embed_cache": { "capacity": 512, "entries": 1, "hits": 0, "misses": 1 }
}
```

Both of these need the **operator** permission when `AUTH_MODE=jwt`
([details](./authentication.md#operator-permission)):

```bash
# Re-drive a non-terminal operation now instead of waiting for the 60 s sweep
curl -s -X POST http://127.0.0.1:3001/api/v2/operations/op-001/retry

# Dead-letter a non-terminal operation
curl -s -X POST http://127.0.0.1:3001/api/v2/operations/op-001/reject \
  -H 'content-type: application/json' -d '{"reason":"payload was wrong"}'
```

Retrying an operation that is already `committed` returns `409`. `reject` is refused
for an `indexed` operation and for the one currently being processed.

Failures that retrying cannot fix, such as an invalid payload, an empty plan or a
plan that changed, reject the operation instead of pausing it, so nothing retries
forever.

## Re-key an agent ID

`rekey_agent_id` rewrites agent ownership of the tenant's memories and TaskStreams to
`to_agent_id`, optionally only those of one `user_id`. Always run it as a dry run
first. It needs the operator permission and a request from the host itself.

```bash
payload='{"dry_run":true,"to_agent_id":"claude-code-v2","user_id":"bob"}'
# compute $hash as above, then submit with "kind":"rekey_agent_id"
```

```json
{ "state": "committed",
  "result": { "dry_run": true, "memories": 0, "task_stream_conflicts": 0,
              "task_streams": 0, "to_agent_id": "claude-code-v2", "user_id": "bob" } }
```

Re-run with `"dry_run": false` (a new operation ID) to apply it.

## The recovery contract

Do not infer success from an HTTP timeout, how long the process has been up, elapsed
time or an attempt count. After any failure:

1. Keep the original operation ID and payload hash.
2. `GET` the receipt.
3. Resume events from the last `sequence` you processed.
4. Let persisted plans continue after a restart.

A repeated `POST` with the same ID and hash is always safe.

The machine-readable contract is the
[OpenAPI 3.1 document](https://github.com/Prometheus-AGS/surreal-memory-server/blob/main/openapi/surreal-memory-v2.openapi.json).
