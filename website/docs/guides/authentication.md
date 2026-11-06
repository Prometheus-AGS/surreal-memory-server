---
id: authentication
title: Authentication and tenancy
sidebar_position: 2
---

# Authentication and tenancy

The HTTP surface (REST, A2A and MCP over HTTP) has two modes. Pick one before you
bind to anything other than your own machine.

| Mode | Who it is for | Credentials | Bind address |
|---|---|---|---|
| `AUTH_MODE=local` (default) | One trusted local client | none | **loopback only**; the server refuses to start otherwise |
| `AUTH_MODE=jwt` | Shared or networked deployments | signed bearer token on every request | any |

The local stdio MCP transport is always the trusted local client and is not
affected by this setting.

:::info Who issues the tokens
The server only **verifies** tokens. UAR/Gate owns identity, membership, grants and
revocation, and issues the token. This server never decides who may do what on its
own, and it ignores `X-User-Id`, `X-Tenant-Id`, `X-Forwarded-For`, payload
ownership fields and the peer IP as proof of identity.
:::

## Turn it on

```bash
export API_HOST=0.0.0.0
export AUTH_MODE=jwt
export AUTH_JWT_ISSUER=gate
export AUTH_JWT_AUDIENCE=surreal-memory
export AUTH_JWT_ALGORITHM=HS256            # HS256 | RS256 | ES256 | EdDSA
export AUTH_JWT_KEY='at-least-32-bytes-of-secret-or-a-public-key-PEM'
# or: export AUTH_JWT_KEY_FILE=/etc/surreal-memory/gate.pub
export AUTH_JWT_MAX_AGE_SECS=300           # optional, the default
```

Startup fails, naming the missing setting, if `AUTH_JWT_ISSUER`,
`AUTH_JWT_AUDIENCE`, `AUTH_JWT_ALGORITHM` or a key is absent, or if an HS256 key
is shorter than 32 bytes. For `RS256`, `ES256` and `EdDSA` the server needs only the
**public** key.

With the wrong combination the server stops immediately and says why:

```text
Error: refusing to start: AUTH_MODE=local serves an unauthenticated API, but
API_HOST=0.0.0.0 is not a loopback address. Bind API_HOST to 127.0.0.1, or set
AUTH_MODE=jwt with the AUTH_JWT_* settings
```

## The token

A bearer token in `Authorization: Bearer <jwt>`.

| Claim | Required | Meaning |
|---|---|---|
| `iss` | yes | must equal `AUTH_JWT_ISSUER` |
| `aud` | yes | must equal `AUTH_JWT_AUDIENCE` |
| `sub` | yes | the authenticated principal; MCP sessions are bound to it |
| `tenant` | yes | tenant id, `[a-z0-9_]{1,48}`; selects the tenant database |
| `iat` | yes | issue time. A token older than `AUTH_JWT_MAX_AGE_SECS`, or issued in the future, is refused |
| `exp` | yes | expiry |
| `ops` | no | permitted operations. `"operator"` allows retry, reject and re-key |
| `user_id` | no | restricts the token to one user's memories (see below) |

**Revocation is freshness.** There is no revocation list. Gate issues short-lived
tokens and stops re-issuing them when a grant is revoked, so a revoked grant stops
working within `AUTH_JWT_MAX_AGE_SECS`. Lower it for a tighter window.

### Minting a token for testing

This needs only `openssl`. In production the token comes from Gate.

```bash
b64() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

# mint_token TENANT SUBJECT [EXTRA_CLAIMS]
mint_token() {
  local tenant=$1 sub=$2 extra=${3:-} now header claims sig
  now=$(date +%s)
  header=$(printf '{"alg":"HS256","typ":"JWT"}' | b64)
  claims=$(printf '{"iss":"gate","aud":"surreal-memory","sub":"%s","tenant":"%s","iat":%d,"exp":%d%s}' \
    "$sub" "$tenant" "$now" $((now + 600)) "$extra" | b64)
  sig=$(printf '%s.%s' "$header" "$claims" | openssl dgst -binary -sha256 -hmac "$AUTH_JWT_KEY" | b64)
  printf '%s.%s.%s\n' "$header" "$claims" "$sig"
}

SERVICE=$(mint_token acme svc-acme)
OPERATOR=$(mint_token acme ops-acme ',"ops":["operator"]')
ALICE=$(mint_token acme alice-app ',"user_id":"alice"')
```

### Calling the API

```bash
curl -s -H "Authorization: Bearer $SERVICE" http://127.0.0.1:3001/api/v1/memory
```

| Situation | Response |
|---|---|
| no token, malformed token, wrong issuer or audience, bad signature, expired, stale or future `iat` | `401` with `WWW-Authenticate: Bearer` |
| valid token, but outside its scope | `403` |
| valid token, tenant storage cannot be opened | `503` |
| `/health`, `/ready` | always public, non-sensitive |

Authentication runs before any handler or storage access. A request with a bad
credential never touches a tenant database.

## One database per tenant

The tenant comes **only** from the verified token and maps to a storage location the
server owns:

| Mode | Where tenant `acme` lives |
|---|---|
| `SURREAL_MODE=server` | namespace `<SURREAL_NAMESPACE>_acme` (default `memory_acme`) |
| `SURREAL_MODE=embedded` | directory `<SURREAL_PATH>/tenants/acme` |

The first request for a tenant creates its database and applies migrations. The
SurrealDB user the server connects as therefore needs the right to create
namespaces. Each tenant also gets its own operation ledger and its own cleanup
worker.

Because the storage is separate, the same IDs mean different things in different
tenants. Two tenants can submit operation `op-1` with different payloads and each
gets its own receipt, events and result. A tenant cannot read, search, change,
delete, get history for, retry or reject anything belonging to another, and
guessing an ID, naming another tenant's scope, or sending empty strings does not
change that.

## User-restricted tokens

A token with `user_id` is for an end user or a thin client that must only see its
own memory.

```bash
# Omitting the owner defaults to the token's user
curl -s -H "Authorization: Bearer $ALICE" http://127.0.0.1:3001/api/v1/memory

# Naming any other owner, including an empty string, is refused
curl -s -H "Authorization: Bearer $ALICE" "http://127.0.0.1:3001/api/v1/memory?user_id=bob"
# {"error":"request names an owner outside the credential's scope"}   (403)
```

Rules for a restricted token:

- It may use **only** `/api/v1/memory` and `/api/v1/search`. Every other route
  returns `403`, because those routes cannot yet honor a per-user restriction.
  A route added later is denied to restricted tokens until it is listed.
- New memories are stamped with the token's `user_id`, so a near-duplicate check
  can only match that user's own memories.
- Another user's memory looks exactly like a missing one (`404`), for read,
  history, update and delete alike.
- Update and delete check ownership **inside the write transaction**, not in an
  earlier read, so a racing change cannot slip through.

A token **without** `user_id` is tenant-wide: appropriate for a service such as UAR
that does its own per-user scoping.

## Operator permission

Retry, reject and re-key change what other callers' work does, so they need
`"ops": ["operator"]` in the verified token. Being on loopback, or behind a proxy,
is never a substitute.

```bash
# An ordinary token is refused
curl -s -o /dev/null -w '%{http_code}\n' -X POST \
  -H "Authorization: Bearer $SERVICE" http://127.0.0.1:3001/api/v2/operations/op-1/retry
# 403

# An operator token of the SAME tenant is allowed (409 here: it already committed)
curl -s -X POST -H "Authorization: Bearer $OPERATOR" \
  http://127.0.0.1:3001/api/v2/operations/op-1/retry
# {"error":"operation 'op-1' is already committed"}
```

An operator of tenant `globex` has no authority over tenant `acme`'s ledger. The
`rekey_agent_id` operation rewrites ownership across the whole tenant store, so it
needs the operator permission **and** a request that provably comes from the host
itself (a loopback peer). See [Durable operations](./durable-operations.md).

## MCP sessions

Streamable HTTP (`/mcp/http`) and the legacy SSE endpoints (`/mcp/sse`,
`/mcp/messages`) bind each session to the `(tenant, sub)` that opened it. Another
principal presenting that session ID, even in the same tenant, is told the session
does not exist. User-restricted tokens cannot use MCP at all.

```bash
curl -s -X POST http://127.0.0.1:3001/mcp/http \
  -H "Authorization: Bearer $SERVICE" \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-03-26","capabilities":{},
        "clientInfo":{"name":"curl","version":"1"}}}' -D -
```

The response carries an `mcp-session-id` header. Send it back on later requests,
with a `Host` header (any real client does).

## Deploying behind a proxy

The Docker Compose files publish on the host loopback and require the `AUTH_JWT_*`
variables:

```yaml
ports:
  - "127.0.0.1:23001:3001"
environment:
  - API_HOST=0.0.0.0
  - AUTH_MODE=jwt
  - AUTH_JWT_ISSUER=${AUTH_JWT_ISSUER:?set AUTH_JWT_ISSUER to the Gate issuer}
  - AUTH_JWT_AUDIENCE=${AUTH_JWT_AUDIENCE:?set AUTH_JWT_AUDIENCE}
  - AUTH_JWT_ALGORITHM=${AUTH_JWT_ALGORITHM:?set AUTH_JWT_ALGORITHM}
  - AUTH_JWT_KEY=${AUTH_JWT_KEY:?set AUTH_JWT_KEY}
```

`surreal-memory.nginx.conf` forwards `Authorization` unchanged and adds no identity
headers of its own. Terminate TLS at the proxy before exposing it beyond the local
machine.

## Known limits

- The `hook` and `statusline` client commands send no token, so they work against a
  local-mode server only. See [CLI and hooks](../reference/cli.md).
- CORS is permissive. That is safe with bearer tokens, but in local mode any web
  page on your machine can reach the loopback API.
- A shared-table design (one database, a `tenant_id` column) is not supported;
  isolation is one database per tenant.
