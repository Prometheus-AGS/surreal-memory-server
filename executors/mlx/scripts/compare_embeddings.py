#!/usr/bin/env python3
"""Certify MLX embeddings against the Candle/CPU BGE implementation."""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
from pathlib import Path

MODEL_ID = "BAAI/bge-small-en-v1.5"
MODEL_REVISION = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"
DIMENSIONS = 384


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candle", required=True, type=Path)
    parser.add_argument("--mlx", required=True, type=Path)
    parser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).parents[1] / "Tests/fixtures/parity-corpus.json",
    )
    return parser.parse_args()


def read_message(process: subprocess.Popen[str]) -> dict[str, object]:
    assert process.stdout is not None
    line = process.stdout.readline()
    if not line:
        stderr = process.stderr.read() if process.stderr else ""
        raise RuntimeError(f"executor closed its output: {stderr}")
    return json.loads(line)


def embed(binary: Path, backend: str, texts: list[str]) -> list[list[float]]:
    environment = os.environ.copy()
    environment.update(
        {
            "EMBEDDING_PROVIDER": "local",
            "LOCAL_EMBEDDING_BACKEND": backend,
            "LOCAL_EMBEDDING_MODEL": MODEL_ID,
            "LOCAL_EMBEDDING_MODEL_REVISION": MODEL_REVISION,
            "LOCAL_EMBEDDING_DIMENSIONS": str(DIMENSIONS),
            "MODEL_CACHE_DIR": str(Path.home() / ".cache/huggingface"),
        }
    )
    process = subprocess.Popen(
        [str(binary), "embedding-executor"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=environment,
    )
    try:
        ready = read_message(process)
        if ready.get("message") != "ready" or ready.get("backend") != backend:
            raise RuntimeError(f"unexpected {backend} ready message: {ready}")
        request = {
            "request_id": 1,
            "operation_id": None,
            "command": {"command": "embed_batch", "texts": texts},
        }
        assert process.stdin is not None
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        while True:
            message = read_message(process)
            if message.get("message") == "progress":
                continue
            if message.get("message") == "failed":
                raise RuntimeError(f"{backend} inference failed: {message.get('error')}")
            if message.get("message") == "completed":
                result = message["result"]
                assert isinstance(result, dict)
                embeddings = result["embeddings"]
                assert isinstance(embeddings, list)
                return embeddings
            raise RuntimeError(f"unexpected {backend} message: {message}")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def dot(left: list[float], right: list[float]) -> float:
    return sum(a * b for a, b in zip(left, right, strict=True))


def norm(vector: list[float]) -> float:
    return math.sqrt(dot(vector, vector))


def cosine(left: list[float], right: list[float]) -> float:
    return dot(left, right) / (norm(left) * norm(right))


def ranking(query: list[float], documents: list[list[float]]) -> list[int]:
    return sorted(
        range(len(documents)),
        key=lambda index: cosine(query, documents[index]),
        reverse=True,
    )


def main() -> None:
    arguments = parse_args()
    corpus = json.loads(arguments.corpus.read_text())
    queries = corpus["queries"]
    documents = corpus["documents"]
    texts = queries + documents
    candle = embed(arguments.candle, "candle", texts)
    mlx = embed(arguments.mlx, "mlx", texts)

    if len(candle) != len(texts) or len(mlx) != len(texts):
        raise SystemExit("embedding count mismatch")
    for backend, vectors in (("candle", candle), ("mlx", mlx)):
        for index, vector in enumerate(vectors):
            if len(vector) != DIMENSIONS:
                raise SystemExit(
                    f"{backend} vector {index} has {len(vector)} dimensions, expected {DIMENSIONS}"
                )
            magnitude = norm(vector)
            if not 0.999 <= magnitude <= 1.001:
                raise SystemExit(f"{backend} vector {index} has norm {magnitude}")

    paired = [cosine(left, right) for left, right in zip(candle, mlx, strict=True)]
    if min(paired) < 0.999:
        raise SystemExit(f"paired cosine minimum {min(paired):.9f} is below 0.999")

    query_count = len(queries)
    candle_documents = candle[query_count:]
    mlx_documents = mlx[query_count:]
    top_one_matches = 0
    top_five_overlap = 0
    for index in range(query_count):
        candle_rank = ranking(candle[index], candle_documents)
        mlx_rank = ranking(mlx[index], mlx_documents)
        top_one_matches += candle_rank[0] == mlx_rank[0]
        top_five_overlap += len(set(candle_rank[:5]) & set(mlx_rank[:5]))

    if top_one_matches != query_count:
        raise SystemExit(f"top-1 parity failed for {query_count - top_one_matches} queries")
    overlap_ratio = top_five_overlap / (query_count * 5)
    if overlap_ratio < 0.95:
        raise SystemExit(f"aggregate top-5 overlap {overlap_ratio:.1%} is below 95%")

    print(
        json.dumps(
            {
                "dimensions": DIMENSIONS,
                "paired_cosine_min": min(paired),
                "status": "ok",
                "top_1_matches": f"{top_one_matches}/{query_count}",
                "top_5_overlap": overlap_ratio,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
