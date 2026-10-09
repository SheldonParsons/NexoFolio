#!/usr/bin/env bash
# Build/publish an immutable backend snapshot. Bash 3.2+ on macOS/Linux.
set +x
set -euo pipefail
export LC_ALL=C
umask 077
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
. "$ROOT/deploy/update-backend.sh"

source_inventory() {
  local root=$1 output=$2 name parent
  git -C "$root" ls-files -z --cached --others --exclude-standard -- apps crates modules tests > "$WORK/git-files"
  printf '%s\n' Cargo.toml Cargo.lock rust-toolchain.toml deploy/Dockerfile > "$WORK/source-names"
  while IFS= read -r -d '' name; do
    case "$name" in */.*|.*) continue ;; esac
    case "$name" in *.rs|*.toml|*.sql|*.json|*.md) ;; *) continue ;; esac
    [[ $name =~ ^[A-Za-z0-9_./-]+$ && $name != *..* && $name != /* ]] || die 'Unsupported source path.'
    [[ -e $root/$name || -L $root/$name ]] || continue
    printf '%s\n' "$name" >> "$WORK/source-names"
  done < "$WORK/git-files"
  sort -u "$WORK/source-names" -o "$WORK/source-names"
  : > "$output"
  while IFS= read -r name; do
    [[ -f $root/$name && ! -L $root/$name ]] || die 'A required source file is missing or is a symlink.'
    parent=$name
    while [[ $parent == */* ]]; do parent=${parent%/*}; [[ ! -L $root/$parent ]] || die 'Symlinked source directories are unsupported.'; done
    printf '%s\t%s\n' "$(hash_file "$root/$name")" "$name" >> "$output"
  done < "$WORK/source-names"
}
put_blob() {
  local source=$1 extension=$2 destination
  BLOB_HASH=$(hash_file "$source")
  destination="$ARTIFACTS/blobs/$BLOB_HASH.$extension"
  if [[ -f $destination ]]; then
    [[ $(hash_file "$destination") == "$BLOB_HASH" ]] || die 'Artifact cache is corrupt.'
  else cp "$source" "$destination"; fi
}
build_release() {
  local digest name info image_platform temporary_tag build_cache source_hash head key snapshot source_path payload_hash release last
  printf 'Taking and verifying a stable source snapshot...\n'
  source_inventory "$ROOT" "$WORK/sources.tsv"
  mkdir "$WORK/source"
  while IFS=$'\t' read -r digest name; do
    mkdir -p "$WORK/source/$(dirname "$name")"
    cp "$ROOT/$name" "$WORK/source/$name"
    [[ $(hash_file "$WORK/source/$name") == "$digest" ]] || die 'Source changed while being copied; retry after edits finish.'
  done < "$WORK/sources.tsv"
  source_inventory "$ROOT" "$WORK/after.tsv"
  cmp -s "$WORK/sources.tsv" "$WORK/after.tsv" || die 'Source changed during snapshot creation; no build was started.'
  awk -F '\t' '$2 ~ /\/adapter\/migrations\/.*\.sql$/ {print}' "$WORK/sources.tsv" > "$WORK/migrations.tsv"
  migrations_check "$WORK/migrations.tsv"
  head=$(git -C "$ROOT" rev-parse --verify HEAD)
  [[ $head =~ ^[a-f0-9]{40,64}$ ]] || die 'Cannot record Git provenance.'
  {
    printf 'recipe\t2\nplatform\t%s\nrust_image\t%s\nruntime_image\t%s\n' "$PLATFORM" "$RUST_IMAGE" "$RUNTIME_IMAGE"
    cat "$WORK/sources.tsv"
  } > "$WORK/build-inputs.tsv"
  source_hash=$(hash_file "$WORK/build-inputs.tsv")
  build_cache="$ARTIFACTS/builds/$source_hash.tsv"
  if [[ $REBUILD == false && -f $build_cache ]]; then
    validate_kv "$build_cache" 'image image_id image_sha image_size'
    IMAGE=$(kv "$build_cache" image); IMAGE_ID=$(kv "$build_cache" image_id)
    IMAGE_SHA=$(kv "$build_cache" image_sha); IMAGE_SIZE=$(kv "$build_cache" image_size)
    valid_hash "$IMAGE_SHA" || die 'Invalid cached image checksum.'
    [[ -f $ARTIFACTS/blobs/$IMAGE_SHA.tar.gz && $(hash_file "$ARTIFACTS/blobs/$IMAGE_SHA.tar.gz") == "$IMAGE_SHA" && $(file_size "$ARTIFACTS/blobs/$IMAGE_SHA.tar.gz") == "$IMAGE_SIZE" ]] || die 'Cached image archive is missing/corrupt; rerun with --rebuild.'
    printf 'Backend inputs unchanged; reusing the verified image archive.\n'
  else
    temporary_tag="nexofolio-backend:build-${source_hash:0:16}"
    STAGE='Linux image build'
    printf 'Building the captured snapshot for %s...\n' "$PLATFORM"
    quiet docker build --platform "$PLATFORM" --build-arg "RUST_IMAGE=$RUST_IMAGE" --build-arg "RUNTIME_IMAGE=$RUNTIME_IMAGE" \
      --label "io.nexofolio.source-sha256=$source_hash" --file "$WORK/source/deploy/Dockerfile" --tag "$temporary_tag" "$WORK/source"
    info=$(docker image inspect --format '{{.Id}} {{.Os}}/{{.Architecture}}' "$temporary_tag" 2>/dev/null)
    read -r IMAGE_ID image_platform <<< "$info"
    [[ $IMAGE_ID =~ ^sha256:[a-f0-9]{64}$ && $image_platform == "$PLATFORM" ]] || die 'Built image platform/ID mismatch.'
    IMAGE="nexofolio-backend:release-${IMAGE_ID:7:16}"
    quiet docker tag "$temporary_tag" "$IMAGE"
    STAGE='Image export'
    docker save "$IMAGE" 2>/dev/null | gzip -n > "$WORK/image.tar.gz"
    put_blob "$WORK/image.tar.gz" tar.gz
    IMAGE_SHA=$BLOB_HASH; IMAGE_SIZE=$(file_size "$WORK/image.tar.gz")
    printf 'image\t%s\nimage_id\t%s\nimage_sha\t%s\nimage_size\t%s\n' "$IMAGE" "$IMAGE_ID" "$IMAGE_SHA" "$IMAGE_SIZE" > "$WORK/build-cache.tsv"
    mv "$WORK/build-cache.tsv" "$build_cache"
  fi
  STAGE='Release packaging'
  printf 'format\t2\nplatform\t%s\nimage\t%s\nimage_id\t%s\nimage_sha\t%s\nimage_size\t%s\nsource_sha\t%s\ngit_head\t%s\n' \
    "$PLATFORM" "$IMAGE" "$IMAGE_ID" "$IMAGE_SHA" "$IMAGE_SIZE" "$source_hash" "$head" > "$WORK/payload.tsv"
  for key in compose env migrations sources readme updater; do
    case "$key" in
      compose) source_path="$ROOT/deploy/offline/compose.yaml" ;;
      env) source_path="$ROOT/deploy/offline/.env.example" ;;
      readme) source_path="$ROOT/deploy/offline/README.md" ;;
      updater) source_path="$ROOT/deploy/update-backend.sh" ;;
      *) source_path="$WORK/$key.tsv" ;;
    esac
    snapshot="$WORK/$key.snapshot"
    digest=$(hash_file "$source_path")
    cp "$source_path" "$snapshot"
    [[ $(hash_file "$snapshot") == "$digest" && $(hash_file "$source_path") == "$digest" ]] || die 'Deployment files changed while packaging.'
    [[ $key != env ]] || template_check "$snapshot"
    put_blob "$snapshot" txt
    printf '%s_sha\t%s\n' "$key" "$BLOB_HASH" >> "$WORK/payload.tsv"
  done
  payload_hash=$(hash_file "$WORK/payload.tsv")
  last="$ARTIFACTS/last-${PLATFORM/\//-}.tsv"
  if [[ -f $last ]]; then
    validate_kv "$last" 'payload_sha release'
    if [[ $(kv "$last" payload_sha) == "$payload_hash" ]]; then
      release=$(kv "$last" release); valid_release "$release" || die 'Invalid cached release ID.'
      PACKAGE_DIR="$ARTIFACTS/releases/$release"
      manifest_check "$PACKAGE_DIR/manifest.tsv"
      awk -F '\t' '$1!="release"' "$PACKAGE_DIR/manifest.tsv" > "$WORK/prior-payload.tsv"
      cmp -s "$WORK/payload.tsv" "$WORK/prior-payload.tsv" || die 'Cached release metadata changed.'
      [[ $(hash_file "$PACKAGE_DIR/update-backend.sh") == "$(kv "$PACKAGE_DIR/manifest.tsv" updater_sha)" ]] || die 'Cached updater changed.'
      printf 'Release contents unchanged: %s\n' "$release"
      return
    fi
  fi
  release="$(date -u +%Y%m%dT%H%M%SZ)-${payload_hash:0:16}"
  PACKAGE_DIR="$ARTIFACTS/releases/$release"
  mkdir "$PACKAGE_DIR"
  { cat "$WORK/payload.tsv"; printf 'release\t%s\n' "$release"; } > "$PACKAGE_DIR/manifest.tsv"
  manifest_check "$PACKAGE_DIR/manifest.tsv"
  cp "$WORK/updater.snapshot" "$PACKAGE_DIR/update-backend.sh"
  printf 'payload_sha\t%s\nrelease\t%s\n' "$payload_hash" "$release" > "$WORK/last.tsv"
  mv "$WORK/last.tsv" "$last"
}

upload_object() {
  local source=$1 relative=$2 immutable=${3:-true} output
  if [[ $immutable == true ]]; then
    if output=$("$OSSUTIL_BIN" stat "$OSS_ROOT/$relative" "${OSS_ARGS[@]}" 2>&1); then
      printf 'Already on OSS: %s\n' "$relative"
      return
    fi
    [[ $output == *NoSuchKey* ]] || die 'Cannot verify OSS object existence; latest pointer was not changed.'
    unset output
  fi
  printf 'Uploading %s...\n' "$relative"
  quiet "$OSSUTIL_BIN" cp "$source" "$OSS_ROOT/$relative" "${OSS_ARGS[@]}" -f
}
publish_release() {
  local key digest source extension release platform
  manifest_check "$PACKAGE_DIR/manifest.tsv"
  release=$(kv "$PACKAGE_DIR/manifest.tsv" release)
  [[ $(basename "$PACKAGE_DIR") == "$release" ]] || die 'Release directory/manifest mismatch.'
  platform=$(kv "$PACKAGE_DIR/manifest.tsv" platform)
  for key in image compose env migrations sources readme updater; do
    digest=$(kv "$PACKAGE_DIR/manifest.tsv" "${key}_sha")
    extension=txt; [[ $key != image ]] || extension=tar.gz
    source="$ARTIFACTS/blobs/$digest.$extension"
    [[ -f $source && $(hash_file "$source") == "$digest" ]] || die 'A local publication artifact is missing or corrupt.'
    if [[ $key == image ]]; then [[ $(file_size "$source") == "$(kv "$PACKAGE_DIR/manifest.tsv" image_size)" ]] || die 'Image size mismatch.'; fi
    upload_object "$source" "blobs/$digest.$extension"
  done
  [[ $(hash_file "$PACKAGE_DIR/update-backend.sh") == "$(kv "$PACKAGE_DIR/manifest.tsv" updater_sha)" ]] || die 'Updater checksum mismatch.'
  upload_object "$PACKAGE_DIR/update-backend.sh" "releases/$release/update-backend.sh"
  upload_object "$PACKAGE_DIR/manifest.tsv" "releases/$release/manifest.tsv"
  upload_object "$PACKAGE_DIR/manifest.tsv" "latest-${platform/\//-}.tsv" false
  printf 'Published: %s\nServer script: %s/releases/%s/update-backend.sh\nServer script SHA-256: %s\n' "$release" "$OSS_HTTP_ROOT" "$release" "$(kv "$PACKAGE_DIR/manifest.tsv" updater_sha)"
  printf 'OSS read permissions must already allow server downloads. No ACL or server configuration was changed.\n'
}

BUILD_ONLY=false; REBUILD=false; PUBLISH_RELEASE=-; PLATFORM=linux/amd64
while (( $# )); do
  case "$1" in
    --build-only) BUILD_ONLY=true; shift ;;
    --rebuild) REBUILD=true; shift ;;
    --platform) [[ $# -ge 2 ]] || die 'Missing platform.'; PLATFORM=$2; shift 2 ;;
    --publish-release) [[ $# -ge 2 ]] || die 'Missing release.'; PUBLISH_RELEASE=$2; shift 2 ;;
    -h|--help) printf 'Usage: ./upload-server.sh [--build-only] [--rebuild] [--platform linux/amd64|linux/arm64] [--publish-release ID]\n'; exit 0 ;;
    *) die 'Unknown argument; use --help.' ;;
  esac
done
valid_platform "$PLATFORM" || die 'Unsupported target architecture.'
if [[ $PUBLISH_RELEASE != - ]]; then
  valid_release "$PUBLISH_RELEASE" || die 'Invalid selected release.'
  [[ $BUILD_ONLY == false && $REBUILD == false ]] || die 'Do not combine --publish-release with build options.'
fi
for command in git docker gzip awk sort cmp od tr wc; do need "$command"; done
if ! command -v sha256sum >/dev/null 2>&1; then need shasum; fi
RUST_IMAGE=${RUST_IMAGE:-rust:1.94.0-bookworm}; RUNTIME_IMAGE=${RUNTIME_IMAGE:-debian:bookworm-slim}
[[ $RUST_IMAGE =~ ^[A-Za-z0-9][A-Za-z0-9_./:@-]*$ && $RUNTIME_IMAGE =~ ^[A-Za-z0-9][A-Za-z0-9_./:@-]*$ ]] || die 'Invalid build image reference.'
ARTIFACTS="$ROOT/target/offline-releases"
mkdir -p "$ARTIFACTS/blobs" "$ARTIFACTS/builds" "$ARTIFACTS/releases"
LOCK_DIR="$ARTIFACTS/.publish-lock"
mkdir "$LOCK_DIR" 2>/dev/null || die 'A publication lock exists; check the active process before removing a stale lock.'
WORK=''; STAGE=initialization
trap 'code=$?; trap - EXIT; [[ -z $WORK ]] || rm -rf -- "$WORK"; rmdir "$LOCK_DIR" 2>/dev/null || true; if (( code )); then printf "%s failed; raw output withheld to protect credentials. Existing releases were retained.\n" "$STAGE" >&2; fi; exit "$code"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
WORK=$(mktemp -d "$ARTIFACTS/.work.XXXXXX")
if [[ $PUBLISH_RELEASE != - ]]; then PACKAGE_DIR="$ARTIFACTS/releases/$PUBLISH_RELEASE"; else build_release; fi
printf 'Local release: %s\n' "$PACKAGE_DIR"
if [[ $BUILD_ONLY == false ]]; then
  STAGE='OSS publication'
  OSSUTIL_BIN=${OSSUTIL_BIN:-ossutil}; need "$OSSUTIL_BIN"
  OSS_BUCKET=${OSS_BUCKET:-asynctest}; OSS_ENDPOINT=${OSS_ENDPOINT:-oss-cn-shenzhen.aliyuncs.com}
  OSS_PATH=${OSS_PATH:-linux_files/nexofolio/backend}; OSS_PATH=${OSS_PATH%/}; OSS_PATH=${OSS_PATH#/}
  [[ $OSS_BUCKET =~ ^[a-z0-9][a-z0-9-]+$ && $OSS_ENDPOINT =~ ^[A-Za-z0-9.-]+$ && $OSS_PATH =~ ^[A-Za-z0-9/_-]+$ && $OSS_PATH != *..* ]] || die 'Invalid OSS destination.'
  OSS_ARGS=(-e "$OSS_ENDPOINT")
  if [[ -n ${OSS_CONFIG_FILE:-} ]]; then OSS_ARGS+=(-c "$OSS_CONFIG_FILE")
  elif [[ -f $HOME/.config/nexofolio/ossutil.config ]]; then OSS_ARGS+=(-c "$HOME/.config/nexofolio/ossutil.config"); fi
  OSS_ROOT="oss://$OSS_BUCKET/$OSS_PATH"; OSS_HTTP_ROOT="https://$OSS_BUCKET.$OSS_ENDPOINT/$OSS_PATH"
  publish_release
fi
