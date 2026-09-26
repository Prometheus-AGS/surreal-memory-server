# Deployment evidence

Recorded 2026-09-20 on the macOS release host under approximately 34.5 GB of
swap use.

The deployed stderr log recorded these startup boundaries:

```text
2026-09-20T18:04:08.179782Z  Initializing storage
2026-09-20T18:04:19.930066Z  Current schema version: v21
2026-09-20T18:04:55.696905Z  Embedding HNSW indexes already match provider dimensions dimension=384
2026-09-20T18:04:55.697190Z  Storage initialized successfully
2026-09-20T18:04:56.289005Z  Starting REST API + HTTP MCP server on http://0.0.0.0:23001
```

Storage initialization therefore completed in 47.5 seconds. Before the
database-side dimension projection, the same startup path transferred roughly
50 MB of full vectors and took about 16 minutes to bind. The projected response
was roughly 1 MB, about a 55-fold reduction.

The installed health check returned:

```text
health_http=200 total_seconds=0.983683
```

The installed server and worker then advanced nine durable receipts from
accepted to completed. The historical backlog was still draining when this
evidence was recorded; this measurement does not claim a doctor-clean queue.
