#!/usr/bin/env bash
# Read-only: no image pulls, builds, container changes, or credential output.
# Manual diagnostic only: export BACKEND_IMAGE and NEXOFOLIO_PLATFORM from the
# release manifest when using an environment file created by update-backend.sh.
set -euo pipefail
if (( $# > 1 )); then
  echo 'Usage: bash preflight.sh [environment-file]' >&2
  exit 2
fi
DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${1:-$DEPLOY_DIR/.env}"
[[ -f "$ENV_FILE" ]] || { echo 'Environment file is missing.' >&2; exit 1; }
command -v docker >/dev/null || { echo 'Docker is missing.' >&2; exit 1; }
docker compose version
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$DEPLOY_DIR/compose.yaml")
# Do not print a resolved Compose configuration: it contains secrets.
"${COMPOSE[@]}" config --quiet
IMAGES="$("${COMPOSE[@]}" config --images | sort -u)"
SERVER_PLATFORM="$(docker info --format '{{.OSType}}/{{.Architecture}}')"
case "$SERVER_PLATFORM" in
  linux/x86_64|linux/amd64) SERVER_PLATFORM=linux/amd64 ;;
  linux/aarch64|linux/arm64) SERVER_PLATFORM=linux/arm64 ;;
  *) echo 'Unsupported Docker daemon platform; verify the target manually.' >&2; exit 1 ;;
esac
while IFS= read -r IMAGE; do
  [[ -n "$IMAGE" ]] || continue
  if ! IMAGE_PLATFORM="$(docker image inspect --format '{{.Os}}/{{.Architecture}}' "$IMAGE" 2>/dev/null)"; then
    echo "Required local image is missing: $IMAGE. Import the release archive first." >&2
    exit 1
  fi
  [[ "$IMAGE_PLATFORM" == "$SERVER_PLATFORM" ]] || {
    echo "Image architecture does not match the Docker daemon: $IMAGE" >&2
    exit 1
  }
  echo "Local image available: $IMAGE ($IMAGE_PLATFORM)"
done <<< "$IMAGES"
echo 'Local image checks passed. Database, credentials, ports, and service connectivity still need verification.'
