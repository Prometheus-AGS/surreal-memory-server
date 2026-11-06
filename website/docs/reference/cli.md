---
id: cli
title: CLI and hooks
sidebar_position: 4
---

# CLI and hooks

`surreal-memory-server` is one binary with a few subcommands. With no subcommand it
serves.

| Command | Does |
|---|---|
| `surreal-memory-server` | serve REST, A2A and MCP (and stdio MCP unless `MCP_STDIO=false`) |
| `surreal-memory-server --version` | print `surreal-memory-server <version>` |
| `surreal-memory-server repair-data [--dry-run\|--apply]` | inspect or repair legacy enum data |
| `surreal-memory-server embedding-executor` | run as the supervised embedding worker (started for you) |
| `surreal-memory-server hook <event>` | Claude Code hook client |
| `surreal-memory-server statusline` | Claude Code status line client |

## Claude Code hooks

The `hook` and `statusline` commands connect the memory server to Claude Code so a
session starts with recalled context and ends by saving its state. They talk **only**
to a running server over HTTP, so they start instantly and never pay for a database
or model start.

They fail open: any problem prints a diagnostic to stderr and the command still exits
`0`, so a stopped or slow server can never block or break Claude Code.

| Event | What it does | Time budget |
|---|---|---|
| `hook session-start` | recalls memories for the project into the agent's context | 2.5 s |
| `hook pre-compact` | saves a session-state memory through the durable operation ledger | 2.5 s |
| `hook session-end` | saves a session-state memory the same way | 0.9 s |
| `statusline` | prints a one-line health summary | 0.5 s |

The server address comes from `SURREAL_MEMORY_URL` (default `http://localhost:23001`).
Stored session-state memories are written with the agent ID `claude-code`.

Wire them up in `.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "surreal-memory-server hook session-start" }] }
    ],
    "PreCompact": [
      { "hooks": [{ "type": "command", "command": "surreal-memory-server hook pre-compact" }] }
    ],
    "SessionEnd": [
      { "hooks": [{ "type": "command", "command": "surreal-memory-server hook session-end" }] }
    ]
  },
  "statusLine": { "type": "command", "command": "surreal-memory-server statusline" }
}
```

```bash
export SURREAL_MEMORY_URL=http://127.0.0.1:3001   # if you did not use the default port
```

`session-start` prints a digest, at most about 1,500 tokens and 8 items, each
trimmed:

```text
## Recalled context (surreal-memory)
- [2026-10-27 · semantic · @project] Deploys go out on Fridays after the freeze check passes …
- [2026-08-03 · procedural · @project] When the migration step times out, re-run it with --apply …
```

and `statusline` prints, for example:

```text
Ψ ok · 12 ops
```

:::warning Local mode only
These commands send no credential. Against a server running with `AUTH_MODE=jwt`
they get `401` and, being fail-open, quietly do nothing. Use them with a
loopback `AUTH_MODE=local` server.
:::

## Repairing legacy data

`repair-data` normalizes enum values that older versions wrote as single-key objects
(`{ Active: {} }`) into strings. Run the dry run first; it changes nothing:

```bash
surreal-memory-server repair-data --dry-run
surreal-memory-server repair-data --apply
```

Any other argument prints `Usage: surreal-memory-server repair-data [--dry-run|--apply]`.
