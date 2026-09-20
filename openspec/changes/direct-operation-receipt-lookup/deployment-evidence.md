# Deployment evidence

Recorded 2026-09-20 on the macOS release host.

The release binary was installed at both runtime paths. `shasum -a 256`
reported the same source hash for each:

```text
71b16086b591f749c808de32883ac9162b7aed3824e93c0e78ec4947534a44a8  /usr/local/bin/surreal-memory-server
71b16086b591f749c808de32883ac9162b7aed3824e93c0e78ec4947534a44a8  /Users/gqadonis/.local/bin/surreal-memory-server
```

Repeated `curl` GETs against `/api/v2/operations/<operation-id>` returned HTTP
200. Observed lookup times were 4.40 seconds, 6.38 seconds, 19.64 seconds on a
cold request, and 6.19 seconds on its warm repeat under 34.5 GB of swap use.
These results established that the direct record lookup is correct but still
host-pressure sensitive.

After the matching 30-second worker request budget was installed, one durable
worker pass moved nine records from `memory/accepted` to `memory/completed`
(344/1932 to 335/1941). The backlog remained in progress when this evidence was
recorded.
