#!/usr/bin/env python3
"""
Prometheus Fabric — Surreal Memory Reconstruction Script
=========================================================
Replays the full knowledge graph seed via the surreal-memory-server REST API.
No MCP dependency — uses direct HTTP calls to localhost:23001.

Usage:
    python3 seed/replay.py [--wipe]

Options:
    --wipe    Delete all existing memories/entities before seeding (fresh start)

The script is idempotent for entities and relations (skips duplicates).
"""

import json
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path

BASE_URL = "http://localhost:23001"
SEED_DIR = Path(__file__).parent

# ── Helpers ────────────────────────────────────────────────────────────────────

def post(path: str, body: dict) -> dict | None:
    url = f"{BASE_URL}{path}"
    data = json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        body_text = e.read().decode()
        print(f"  ✗ HTTP {e.code} on {path}: {body_text[:120]}")
        return None
    except Exception as e:
        print(f"  ✗ Error on {path}: {e}")
        return None

def get(path: str) -> dict | list | None:
    url = f"{BASE_URL}{path}"
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return json.loads(resp.read())
    except Exception as e:
        print(f"  ✗ GET {path}: {e}")
        return None

def check_health():
    result = get("/health")
    if not result or result.get("status") != "ok":
        print("✗ Server not healthy at", BASE_URL)
        print("  Start it with: cd /Users/gqadonis/Projects/references/surreal-memory-server && docker compose up -d")
        sys.exit(1)
    print(f"✓ Server healthy — v{result.get('version', '?')}")

# ── Entities ───────────────────────────────────────────────────────────────────

def seed_entities(entities: list) -> int:
    print(f"\n── Entities ({len(entities)}) ──────────────────────────────")
    ok = 0
    for e in entities:
        result = post("/api/entities", {
            "name": e["name"],
            "entity_type": e["entity_type"],
            "observations": e["observations"]
        })
        if result:
            print(f"  ✓ {e['entity_type']:16} {e['name']}")
            ok += 1
        else:
            # Try adding observations to existing entity instead
            result2 = post(f"/api/entities/{urllib.parse.quote(e['name'])}/observations",
                           {"observations": e["observations"]})
            if result2:
                print(f"  ~ {e['entity_type']:16} {e['name']} (updated)")
                ok += 1
            else:
                print(f"  ✗ {e['entity_type']:16} {e['name']} (skipped)")
        time.sleep(0.05)
    return ok

# ── Relations ──────────────────────────────────────────────────────────────────

def seed_relations(relations: list) -> int:
    print(f"\n── Relations ({len(relations)}) ─────────────────────────────")
    ok = 0
    for r in relations:
        result = post("/api/relations", {
            "from": r["from"],
            "to": r["to"],
            "relation_type": r["relation_type"]
        })
        if result:
            print(f"  ✓ {r['from']} --{r['relation_type']}--> {r['to']}")
            ok += 1
        else:
            print(f"  ~ {r['from']} --{r['relation_type']}--> {r['to']} (may already exist)")
        time.sleep(0.03)
    return ok

# ── Mindmaps ───────────────────────────────────────────────────────────────────

def seed_mindmaps(mindmaps: list) -> int:
    print(f"\n── Mindmaps ({len(mindmaps)}) ──────────────────────────────")
    ok = 0
    for mm in mindmaps:
        # Create mindmap
        result = post("/api/mindmaps", {
            "name": mm["name"],
            "map_type": mm["map_type"],
            "root_label": mm["root_label"],
            "description": mm.get("description")
        })
        if result:
            print(f"  ✓ Created mindmap: {mm['name']}")
        else:
            print(f"  ~ Mindmap {mm['name']} may already exist — adding nodes")

        # Add nodes
        node_ok = 0
        for node in mm.get("nodes", []):
            nr = post(f"/api/mindmaps/{urllib.parse.quote(mm['name'])}/nodes", {
                "node_id": node["node_id"],
                "label": node["label"],
                "parent_id": node.get("parent_id"),
                "color": node.get("color"),
                "node_type": node.get("node_type"),
                "metadata": node.get("metadata")
            })
            if nr:
                node_ok += 1
            time.sleep(0.05)
        print(f"    nodes: {node_ok}/{len(mm.get('nodes', []))}")

        # Add edges
        edge_ok = 0
        for edge in mm.get("edges", []):
            er = post(f"/api/mindmaps/{urllib.parse.quote(mm['name'])}/edges", {
                "from_id": edge["from_id"],
                "to_id": edge["to_id"],
                "label": edge.get("label"),
                "directed": edge.get("directed", True)
            })
            if er:
                edge_ok += 1
            time.sleep(0.03)
        if mm.get("edges"):
            print(f"    edges: {edge_ok}/{len(mm.get('edges', []))}")
        ok += 1
    return ok

# ── Task Streams + Memories ────────────────────────────────────────────────────

def seed_task_streams(task_streams: list, memories: list) -> int:
    print(f"\n── Task Streams ({len(task_streams)}) ──────────────────────")
    ok = 0
    for ts in task_streams:
        result = post("/api/task-streams", {
            "name": ts["name"],
            "description": ts.get("description")
        })
        if result:
            print(f"  ✓ Created stream: {ts['name']}")
            ok += 1
        else:
            print(f"  ~ Stream {ts['name']} may already exist")
        time.sleep(0.05)

    print(f"\n── Memories ({len(memories)}) ──────────────────────────────")
    mem_ok = 0
    for m in memories:
        result = post(f"/api/task-streams/{urllib.parse.quote(m['stream'])}/memories", {
            "content": m["content"]
        })
        if result:
            mem_ok += 1
            print(f"  ✓ [{m['stream']}] {m['content'][:60]}...")
        time.sleep(0.1)
    print(f"  Memories written: {mem_ok}/{len(memories)}")
    return ok


# ── Main ───────────────────────────────────────────────────────────────────────

def main():
    import urllib.parse  # noqa: needed inside functions above

    wipe = "--wipe" in sys.argv

    print("=" * 60)
    print("Prometheus Fabric — Surreal Memory Reconstruction")
    print("=" * 60)

    check_health()

    if wipe:
        print("\n⚠  --wipe flag detected — this would delete all data.")
        print("   (Wipe not implemented in this script; do it manually.)")

    # Load seed files
    kg   = json.loads((SEED_DIR / "knowledge_graph_seed.json").read_text())
    mm   = json.loads((SEED_DIR / "mindmap_seed.json").read_text())
    mems = json.loads((SEED_DIR / "memories_seed.json").read_text())

    # Seed everything
    ent_ok = seed_entities(kg["entities"])
    rel_ok = seed_relations(kg["relations"])
    mm_ok  = seed_mindmaps(mm["mindmaps"])
    ts_ok  = seed_task_streams(mems["task_streams"], mems["memories"])

    print("\n" + "=" * 60)
    print("Reconstruction complete")
    print(f"  Entities:     {ent_ok}/{len(kg['entities'])}")
    print(f"  Relations:    {rel_ok}/{len(kg['relations'])}")
    print(f"  Mindmaps:     {mm_ok}/{len(mm['mindmaps'])}")
    print(f"  Task streams: {ts_ok}/{len(mems['task_streams'])}")
    print("=" * 60)


if __name__ == "__main__":
    import urllib.parse
    main()
