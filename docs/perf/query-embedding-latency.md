# Query embedding latency (change-tlh-05, task 1)

Measured on 2026-10-05, before any query-embedding cache existed. Run with `cargo test --test query_embedding_latency -- --ignored --nocapture` against the installed MLX executor (`/usr/local/bin/surreal-memory-mlx-executor`, BAAI/bge-small-en-v1.5, 384 dimensions) on the operator's Mac.

**Method:**
- 30 sequential embeddings of distinct query texts, after one warm-up embedding.
- "Loaded" repeats the 30 while 8 writer tasks embed continuously through the same supervised executor.

**Machine load:** 1-minute load average 56.03 at the start (5-minute 81.80). The machine was not idle in absolute terms; it never is during a phase.

## Results

IDLE_P50_SECONDS=0.010
IDLE_P95_SECONDS=0.014
LOADED_P50_SECONDS=0.103
LOADED_P95_SECONDS=0.340

The warm-up (first embedding after the executor started) took 0.573 s.

## Reading

- The stop rule (idle p95 > 2.0 s) does not trigger: one embedding is about 14 ms idle.
- Under executor contention, p95 rises to 0.34 s. A non-lead SubagentStart recall makes 4 search calls, so at the loaded p95 about 1.4 s of its 2.8 s deadline goes to repeated embeddings. A cache that embeds once saves about 1 s in that case.
- The embedding is **not** the only cost. Last phase's watchdog misses happened at load averages around 250, well above this run, and they also include process start-up, the database query and pk. The cache removes the repeated part; it does not by itself guarantee the deadline.
