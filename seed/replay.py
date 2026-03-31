#!/usr/bin/env python3
"""
Prometheus Fabric — Surreal Memory Reconstruction Script
=========================================================
Replays the full knowledge graph seed via the surreal-memory-server REST API.
No MCP dependency — uses direct HTTP calls to localhost:23001.

API routes (v0.1.0):
  POST /api/v1/entities/batch         — create entities in bulk
  POST /api/v1/entities/relations/batch — create relations in bulk
  POST /api/v1/mindmaps               — create mindmap
  POST /api/v1/mindmaps/{name}/nodes  — add node
  POST /api/v1/mindmaps/{name}/edges  — add edge
  POST /api/v1/taskstreams            — create task stream
  POST /api/v1/taskstreams/{name}/memories — add memory to stream

Usage:
    python3 seed/replay.py
"""

import json, sys, time, urllib.request, urllib.error, urllib.parse
from pathlib import Path

BASE_URL = "http://localhost:23001"
SEED_DIR = Path(__file__).parent

def post(path, body):
    url = f"{BASE_URL}{path}"
    data = json.dumps(body).encode()
    req = urllib.request.Request(url, data=data,
          headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        msg = e.read().decode()[:160]
        print(f"    HTTP {e.code}: {msg}")
        return None
    except Exception as e:
        print(f"    ERR: {e}")
        return None

def check_health():
    req = urllib.request.Request(f"{BASE_URL}/health")
    with urllib.request.urlopen(req, timeout=8) as r:
        d = json.loads(r.read())
    assert d.get("status") == "ok", f"unhealthy: {d}"
    print(f"✓ Server healthy — v{d.get('version','?')}")

def seed_entities(entities):
    print(f"\n── Entities ({len(entities)}) ──────────────────────")
    result = post("/api/v1/entities/batch", entities)
    if result:
        print(f"  ✓ {len(result)} entities written")
    else:
        print("  batch failed — trying one-by-one")
        ok = 0
        for e in entities:
            r = post("/api/v1/entities", e)
            if r: ok += 1
            time.sleep(0.05)
        print(f"  ✓ {ok}/{len(entities)}")

def seed_relations(relations):
    print(f"\n── Relations ({len(relations)}) ─────────────────────")
    result = post("/api/v1/entities/relations/batch", relations)
    if result:
        print(f"  ✓ {len(result)} relations written")
    else:
        print("  batch failed — trying one-by-one")
        ok = 0
        for r in relations:
            res = post("/api/v1/entities/relations", r)
            if res: ok += 1
            time.sleep(0.03)
        print(f"  ✓ {ok}/{len(relations)}")

def seed_mindmaps(mindmaps):
    print(f"\n── Mindmaps ({len(mindmaps)}) ──────────────────────")
    for mm in mindmaps:
        name = mm["name"]
        r = post("/api/v1/mindmaps", {
            "name": name, "map_type": mm["map_type"],
            "root_label": mm["root_label"],
            "description": mm.get("description")
        })
        if r:   print(f"  ✓ created: {name}")
        else:   print(f"  ~ exists:  {name}")

        node_ok = 0
        for node in mm.get("nodes", []):
            nr = post(f"/api/v1/mindmaps/{urllib.parse.quote(name)}/nodes", {
                "node_id":   node["node_id"],
                "label":     node["label"],
                "parent_id": node.get("parent_id"),
                "color":     node.get("color"),
                "node_type": node.get("node_type"),
                "metadata":  node.get("metadata"),
            })
            if nr: node_ok += 1
            time.sleep(0.06)
        print(f"    nodes {node_ok}/{len(mm.get('nodes', []))}")

        edge_ok = 0
        for edge in mm.get("edges", []):
            er = post(f"/api/v1/mindmaps/{urllib.parse.quote(name)}/edges", {
                "from_id":  edge["from_id"],
                "to_id":    edge["to_id"],
                "label":    edge.get("label"),
                "directed": edge.get("directed", True),
            })
            if er: edge_ok += 1
            time.sleep(0.04)
        if mm.get("edges"):
            print(f"    edges {edge_ok}/{len(mm.get('edges', []))}")

def seed_task_streams(streams, memories):
    print(f"\n── Task Streams ({len(streams)}) ────────────────────")
    for ts in streams:
        r = post("/api/v1/taskstreams", {"name": ts["name"], "description": ts.get("description")})
        print(f"  {'✓' if r else '~'} {ts['name']}")
        time.sleep(0.05)

    print(f"\n── Memories ({len(memories)}) ──────────────────────")
    ok = 0
    for m in memories:
        r = post(f"/api/v1/taskstreams/{urllib.parse.quote(m['stream'])}/memories",
                 {"content": m["content"]})
        if r:
            ok += 1
            print(f"  ✓ [{m['stream']}] {m['content'][:70]}...")
        time.sleep(0.12)
    print(f"  Total: {ok}/{len(memories)}")

def main():
    print("=" * 58)
    print("Prometheus Fabric — Surreal Memory Reconstruction")
    print("=" * 58)
    check_health()

    kg   = json.loads((SEED_DIR / "knowledge_graph_seed.json").read_text())
    mm   = json.loads((SEED_DIR / "mindmap_seed.json").read_text())
    mems = json.loads((SEED_DIR / "memories_seed.json").read_text())

    seed_entities(kg["entities"])
    seed_relations(kg["relations"])
    seed_mindmaps(mm["mindmaps"])
    seed_task_streams(mems["task_streams"], mems["memories"])

    print("\n" + "=" * 58)
    print("Reconstruction complete ✓")
    print("=" * 58)

if __name__ == "__main__":
    main()
