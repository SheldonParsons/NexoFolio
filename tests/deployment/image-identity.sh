#!/usr/bin/env bash
# Real Docker-save-shaped archives, simulated Docker inspect results; no daemon.
set -euo pipefail
export LC_ALL=C
umask 077
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
. "$ROOT/deploy/update-backend.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf -- "$TEST_ROOT"' EXIT
DEPLOY_ROOT="$TEST_ROOT/server"
RELEASE_DIR="$DEPLOY_ROOT/releases/fixture"
mkdir -p "$RELEASE_DIR" "$DEPLOY_ROOT/cache" "$TEST_ROOT/pack/blobs/sha256"
BACKEND_IMAGE=nexofolio-backend:release-1111111111111111
EXPECTED_IMAGE_ID="sha256:$(printf '%064d' 0 | tr 0 1)"
NEXOFOLIO_PLATFORM=linux/amd64
printf '{"architecture":"amd64","os":"linux","config":{"Env":["FIXTURE=1"]},"rootfs":{"type":"layers","diff_ids":[]}}' > "$TEST_ROOT/config.json"
CONFIG_SHA=$(hash_file "$TEST_ROOT/config.json")
cp "$TEST_ROOT/config.json" "$TEST_ROOT/pack/blobs/sha256/$CONFIG_SHA"
cp "$TEST_ROOT/config.json" "$TEST_ROOT/pack/$CONFIG_SHA.json"
PASS=0
passed() { PASS=$((PASS+1)); printf 'PASS %s\n' "$1"; }
docker() {
  case "$1 $2" in
    'image inspect') printf '%s %s\n' "$TEST_IMAGE_ID" "$TEST_PLATFORM" ;;
    'inspect --format') printf '%s\n' "$TEST_CONTAINER_ID" ;;
    *) return 1 ;;
  esac
}
dc() {
  case "$1" in
    ps) printf '%064d\n' 1 ;;
    exec) if [[ ${!#} == */live ]]; then printf '{"status":"alive"}'; else printf '{"status":"ready"}'; fi ;;
    *) return 1 ;;
  esac
}
write_archive() {
  printf '[{"Layers":[],"RepoTags":["%s"],"Config":"%s"}]\n' "$2" "$1" > "$TEST_ROOT/pack/manifest.json"
  tar -cf - -C "$TEST_ROOT/pack" manifest.json blobs "$CONFIG_SHA.json" | gzip -n > "$TEST_ROOT/image.tar.gz"
  ARCHIVE_SHA=$(hash_file "$TEST_ROOT/image.tar.gz")
  ARCHIVE="$DEPLOY_ROOT/cache/$ARCHIVE_SHA.tar.gz"
  cp "$TEST_ROOT/image.tar.gz" "$ARCHIVE"
  printf 'image_sha\t%s\nimage_size\t%s\n' "$ARCHIVE_SHA" "$(file_size "$ARCHIVE")" > "$RELEASE_DIR/manifest.tsv"
}

write_archive "blobs/sha256/$CONFIG_SHA" "$BACKEND_IMAGE"
TEST_PLATFORM=linux/amd64
TEST_IMAGE_ID=$EXPECTED_IMAGE_ID
TEST_CONTAINER_ID=$EXPECTED_IMAGE_ID
image_matches_release
verify_health
passed 'Containerd index identity remains supported'

TEST_IMAGE_ID="sha256:$CONFIG_SHA"
TEST_CONTAINER_ID=$TEST_IMAGE_ID
image_matches_release
verify_health
[[ $VERIFIED_IMAGE_ID == "$TEST_IMAGE_ID" ]]
passed 'Classic config identity and its running container are accepted from the verified archive'

TEST_CONTAINER_ID=$EXPECTED_IMAGE_ID
if verify_health; then die 'An unrelated running container image was accepted.'; fi
TEST_CONTAINER_ID=$TEST_IMAGE_ID
passed 'Health verification still rejects a different container image'

TEST_IMAGE_ID="sha256:$(printf '%064d' 9)"
if image_matches_release; then die 'Unknown image ID was accepted.'; fi
TEST_IMAGE_ID="sha256:$CONFIG_SHA"
passed 'Unknown image IDs remain rejected'

TEST_PLATFORM=linux/arm64
if image_matches_release; then die 'Wrong architecture was accepted.'; fi
TEST_PLATFORM=linux/amd64
passed 'Architecture mismatch remains rejected'

printf x >> "$ARCHIVE"
if image_matches_release; then die 'Corrupted archive was accepted.'; fi
passed 'Corrupted archive cannot authorize a legacy ID'

write_archive "$CONFIG_SHA.json" "$BACKEND_IMAGE"
image_matches_release
passed 'Legacy Docker save config filenames are supported'

write_archive "blobs/sha256/$CONFIG_SHA" nexofolio-backend:unrelated
if image_matches_release; then die 'Wrong saved tag was accepted.'; fi
passed 'The config digest must belong to the exact published tag'

write_archive ../config.json "$BACKEND_IMAGE"
if image_matches_release; then die 'Traversal config path was accepted.'; fi
passed 'Untrusted archive paths are never extracted'

printf 'changed configuration' > "$TEST_ROOT/pack/blobs/sha256/$CONFIG_SHA"
write_archive "blobs/sha256/$CONFIG_SHA" "$BACKEND_IMAGE"
if image_matches_release; then die 'Mismatched config content hash was accepted.'; fi
passed 'The image configuration bytes must match their content digest'

printf '%s image identity checks passed.\n' "$PASS"
