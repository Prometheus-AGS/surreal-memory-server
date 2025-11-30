#!/bin/bash
set -euo pipefail

echo "🔨 Building Rust Memory MCP Server for Apple Silicon (M1/M2/M3)"
echo ""

FEATURE_FLAGS="${FEATURE_FLAGS:-embedded,metal,local-embeddings}"
export FEATURE_FLAGS
export RUSTFLAGS="-Dwarnings ${RUSTFLAGS:-}"

# Check if we're on ARM64
if [[ $(uname -m) != "arm64" ]]; then
    echo "⚠️  Warning: Not running on ARM64 architecture"
fi

# Clean previous builds
echo "🧹 Cleaning previous builds..."
cargo clean

# Ensure fmt, clippy (warnings as errors), and tests pass before the release build
echo "✅ Running quality gate checks..."
./scripts/quality-check.sh

# Build with Metal support
echo "🚀 Building with Metal GPU support..."
cargo build --release --no-default-features --features "${FEATURE_FLAGS}"

# Check if build succeeded
if [ $? -eq 0 ]; then
    echo ""
    echo "✅ Build successful!"
    echo ""
    echo "📦 Binary location: ./target/release/surreal-memory-server"

    # Show binary info
    echo ""
    echo "📊 Binary info:"
    ls -lh ./target/release/surreal-memory-server
    file ./target/release/surreal-memory-server

    # Test run
    echo ""
    echo "🧪 Testing binary..."
    ./target/release/surreal-memory-server --version 2>/dev/null || echo "Ready to run!"
else
    echo ""
    echo "❌ Build failed"
    exit 1
fi
