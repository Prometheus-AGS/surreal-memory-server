#!/usr/bin/env bash
set -euo pipefail

# Enforce warnings-as-errors and run the quality gate across formatting, linting, and tests.
FEATURE_FLAGS="${FEATURE_FLAGS:-embedded,metal}"
export RUSTFLAGS="-Dwarnings ${RUSTFLAGS:-}"

echo "Running quality gate with features: ${FEATURE_FLAGS}"
echo "1) cargo fmt --all --check"
cargo fmt --all --check

echo "2) cargo clippy --all-targets --no-default-features --features \"${FEATURE_FLAGS}\" -- -D warnings"
cargo clippy --all-targets --no-default-features --features "${FEATURE_FLAGS}" -- -D warnings

echo "3) cargo test --all-targets --no-default-features --features \"${FEATURE_FLAGS}\""
cargo test --all-targets --no-default-features --features "${FEATURE_FLAGS}"
