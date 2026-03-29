#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

IMAGE_TAG="surreal-memory-server-builder:local"
TARGET_BINARY="target/release/surreal-memory-server"
HOST_ARCH="$(uname -m)"

case "${HOST_ARCH}" in
  arm64|aarch64)
    DEFAULT_PLATFORM="linux/arm64"
    ;;
  x86_64|amd64)
    DEFAULT_PLATFORM="linux/amd64"
    ;;
  *)
    echo "Unsupported host architecture: ${HOST_ARCH}" >&2
    exit 1
    ;;
esac

TARGET_PLATFORM="${LOCAL_DOCKER_PLATFORM:-${DEFAULT_PLATFORM}}"

echo "Building ${TARGET_PLATFORM} binary inside Docker..."
docker buildx build \
  --platform "${TARGET_PLATFORM}" \
  --target builder \
  --build-arg PREBUILD_DEPS=0 \
  --load \
  -t "${IMAGE_TAG}" \
  .

echo "Exporting binary to ${TARGET_BINARY}..."
mkdir -p "$(dirname "${TARGET_BINARY}")"
container_id="$(docker create "${IMAGE_TAG}")"
trap 'docker rm -f "${container_id}" >/dev/null 2>&1 || true' EXIT
docker cp "${container_id}:/app/target/release/surreal-memory-server" "${TARGET_BINARY}"
chmod +x "${TARGET_BINARY}"

echo "${TARGET_PLATFORM} binary ready at ${TARGET_BINARY}"
