# Current Waypoint

- **Phase**: task-stream-reliability
- **Status**: planned
- **Change backend**: OpenSpec

## Ordered changes (execute strictly in order)

1. `fix-taskstream-scope-bugs` — CRITICAL bug fixes (C-1 data loss, C-2 cross-tenant access)
2. `harden-taskstream-correctness` — H-2/H-3/M-1/M-2 correctness + test suite
3. `add-taskstream-steps` — H-1 first-class step abstraction (idempotency, resumability)

## Next step

`/kbd-execute fix-taskstream-scope-bugs`
