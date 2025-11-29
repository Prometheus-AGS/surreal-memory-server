#!/bin/bash
set -e

echo "🔨 Building Rust Memory MCP Server for Apple Silicon (M1/M2/M3)"
echo ""

# Check if we're on ARM64
if [[ $(uname -m) != "arm64" ]]; then
    echo "⚠️  Warning: Not running on ARM64 architecture"
fi

# Clean previous builds
echo "🧹 Cleaning previous builds..."
cargo clean

# Build with Metal support
echo "🚀 Building with Metal GPU support..."
cargo build --release --no-default-features --features "embedded,metal"

# Check if build succeeded
if [ $? -eq 0 ]; then
    echo ""
    echo "✅ Build successful!"
    echo ""
    echo "📦 Binary location: ./target/release/rust-memory-mcp"

    # Show binary info
    echo ""
    echo "📊 Binary info:"
    ls -lh ./target/release/rust-memory-mcp
    file ./target/release/rust-memory-mcp

    # Test run
    echo ""
    echo "🧪 Testing binary..."
    ./target/release/rust-memory-mcp --version 2>/dev/null || echo "Ready to run!"
else
    echo ""
    echo "❌ Build failed"
    exit 1
fi
