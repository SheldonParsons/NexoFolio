#!/usr/bin/env bash
# Stateful shell integration tests using only temporary repos and fake commands.
set -euo pipefail
export LC_ALL=C
umask 077
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
. "$ROOT/deploy/update-backend.sh"
TEST_ROOT=$(mktemp -d)
TEST_ROOT=$(cd "$TEST_ROOT" && pwd -P)
trap 'rm -rf -- "$TEST_ROOT"' EXIT
REAL_GIT=$(command -v git); REAL_CP=$(command -v cp)
export REAL_GIT REAL_CP
export FAKE_ROOT="$TEST_ROOT/runtime" FAKE_OSS="$TEST_ROOT/oss"
mkdir -p "$TEST_ROOT/bin" "$FAKE_ROOT" "$FAKE_OSS"
for tool in docker ossutil curl git cp python python3 jq; do ln -s "$ROOT/tests/deployment/fake-command.sh" "$TEST_ROOT/bin/$tool"; done
export PATH="$TEST_ROOT/bin:$PATH"
FIXTURE="$TEST_ROOT/project"
mkdir -p "$FIXTURE/deploy/offline" "$FIXTURE/apps" "$FIXTURE/modules/access/adapter/migrations"
cp "$ROOT/upload-server.sh" "$FIXTURE/"
cp "$ROOT/deploy/update-backend.sh" "$FIXTURE/deploy/"
cp "$ROOT/deploy/offline/compose.yaml" "$ROOT/deploy/offline/.env.example" "$ROOT/deploy/offline/README.md" "$FIXTURE/deploy/offline/"
for file in Cargo.toml Cargo.lock rust-toolchain.toml deploy/Dockerfile apps/main.rs; do printf 'fixture source\n' > "$FIXTURE/$file"; done
printf 'fixture migration\n' > "$FIXTURE/modules/access/adapter/migrations/202601010001_init.sql"
printf 'MODEL_KEY=fixture-sensitive-source\n' > "$FIXTURE/apps/.env"
"$REAL_GIT" init -q "$FIXTURE"
"$REAL_GIT" -C "$FIXTURE" add apps/.env
PASS=0
passed() { PASS=$((PASS+1)); printf 'PASS %s\n' "$1"; }
invoke() { if ! bash "$@" > "$TEST_ROOT/last.log" 2>&1; then cat "$TEST_ROOT/last.log" >&2; die "Unexpected command failure: $1"; fi; }
expect_fail() { if bash "$@" > "$TEST_ROOT/last.log" 2>&1; then die 'Expected command failure.'; fi; }
count() { if [[ -f $FAKE_ROOT/events ]]; then awk -v name="$1" '$0==name {n++} END {print n+0}' "$FAKE_ROOT/events"; else printf 0; fi; }
state() { kv "$SERVER/state.tsv" "$1"; }
new_release() {
  invoke "$FIXTURE/upload-server.sh" --build-only
  BUILT=$(kv "$FIXTURE/target/offline-releases/last-linux-amd64.tsv" release)
  invoke "$FIXTURE/upload-server.sh" --publish-release "$BUILT"
}

if docker compose run --pull never migrate > "$TEST_ROOT/flag-check.log" 2>&1; then die 'The v2.27 command fixture accepted an unsupported flag.'; fi
grep -q 'unknown flag: --pull' "$TEST_ROOT/flag-check.log"
passed 'Compose v2.27 run rejects the unsupported --pull flag'

invoke "$FIXTURE/upload-server.sh" --build-only
A=$(kv "$FIXTURE/target/offline-releases/last-linux-amd64.tsv" release)
MANIFEST="$FIXTURE/target/offline-releases/releases/$A/manifest.tsv"
manifest_check "$MANIFEST"
! grep -q '\.env' "$FIXTURE/target/offline-releases/blobs/$(kv "$MANIFEST" sources_sha).txt"
invoke "$FIXTURE/upload-server.sh" --build-only
[[ $(count build) == 1 && $(kv "$FIXTURE/target/offline-releases/last-linux-amd64.tsv" release) == "$A" ]]
passed 'Stable source snapshots exclude tracked secrets and reuse unchanged builds/releases'

invoke "$FIXTURE/upload-server.sh" --publish-release "$A"
[[ $(tail -n 1 "$FAKE_ROOT/events") == 'oss-copy latest-linux-amd64.tsv' ]]
before=$(awk '/^oss-copy / {n++} END {print n+0}' "$FAKE_ROOT/events")
invoke "$FIXTURE/upload-server.sh" --publish-release "$A"
after=$(awk '/^oss-copy / {n++} END {print n+0}' "$FAKE_ROOT/events")
[[ $((after-before)) == 1 ]]
passed 'Unchanged immutable OSS objects are skipped and latest is written last'

export FAKE_FAIL=oss-stat
expect_fail "$FIXTURE/upload-server.sh" --publish-release "$A"
! grep -q 'fixture-sensitive-error' "$TEST_ROOT/last.log"
unset FAKE_FAIL
passed 'OSS permission/lookup errors stop publication without exposing raw errors'

cp "$MANIFEST" "$TEST_ROOT/bad.tsv"
printf 'release\t$(touch %s)\n' "$TEST_ROOT/injected" >> "$TEST_ROOT/bad.tsv"
if (manifest_check "$TEST_ROOT/bad.tsv") > /dev/null 2>&1; then die 'Invalid manifest accepted.'; fi
[[ ! -e $TEST_ROOT/injected ]]
passed 'Duplicate/injected metadata is rejected without execution'

SERVER="$TEST_ROOT/server"
invoke "$ROOT/deploy/update-backend.sh" init --deploy-dir "$SERVER"
[[ -f $SERVER/.env && ! -f $SERVER/state.tsv ]]
sed 's#^NEXOFOLIO_ZENTAO_BASE_URL=.*#NEXOFOLIO_ZENTAO_BASE_URL=http://zentao.test#' "$SERVER/.env" > "$TEST_ROOT/config.env"
mv "$TEST_ROOT/config.env" "$SERVER/.env"
printf '\nNEXOFOLIO_TEST_LITERAL=$(touch %s)\n' "$TEST_ROOT/env-injected" >> "$SERVER/.env"
private_file "$SERVER/.env"
passed 'Init creates protected local credentials without starting containers'

printf 'unrelated-owner' > "$FAKE_ROOT/volume-owner"
expect_fail "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER"
[[ ! -f $SERVER/state.tsv && $(count volume-create) == 0 ]]
rm "$FAKE_ROOT/volume-owner"
passed 'An existing unrelated volume is never adopted'

invoke "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER"
[[ $(state current) == "$A" && $(state pending) == - && -f $SERVER/$(state backup) ]]
[[ ! -e $TEST_ROOT/env-injected ]]
backup_count=$(count backup); api_count=$(count api-up)
invoke "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER"
[[ $(count backup) == "$backup_count" && $(count api-up) == "$api_count" ]]
passed 'First deployment verifies health; repeated update preserves data and avoids restart'

printf 'second source\n' >> "$FIXTURE/apps/main.rs"
new_release; B=$BUILT
export FAKE_FAIL=backup
migration_count=$(count migrate)
expect_fail "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER"
[[ $(state current) == "$A" && $(state pending) == "$B" && $(count migrate) == "$migration_count" ]]
[[ ! -f $FAKE_ROOT/api-running ]]
unset FAKE_FAIL
passed 'Backup failure prevents migration and retains the prior successful version'

export FAKE_FAIL=health
expect_fail "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER" --release "$B"
[[ $(state current) == "$A" && ! -f $FAKE_ROOT/api-running ]]
original_backup=$(state backup); backup_count=$(count backup)
unset FAKE_FAIL
printf '\nNEXOFOLIO_LOG=debug\n' >> "$SERVER/.env"
# Duplicate settings are rejected rather than silently overriding.
expect_fail "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER" --release "$B"
sed '$d' "$SERVER/.env" > "$TEST_ROOT/config.env"; mv "$TEST_ROOT/config.env" "$SERVER/.env"
invoke "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER" --release "$B"
[[ $(state current) == "$B" && $(state backup) == "$original_backup" && $(count backup) == "$backup_count" ]]
passed 'Failed candidate health stops the API; retry preserves the original backup'

invoke "$ROOT/deploy/update-backend.sh" rollback --deploy-dir "$SERVER"
[[ $(state current) == "$A" && $(state previous) == "$B" ]]
passed 'A previous release with identical migrations can be rolled back'

printf 'second migration\n' > "$FIXTURE/modules/access/adapter/migrations/202602010001_next.sql"
new_release; C=$BUILT
export FAKE_FAIL=migrate
api_count=$(count api-up)
expect_fail "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER"
grep -q 'Backend command failed: action=migrate exit=1' "$TEST_ROOT/last.log"
! grep -q 'fixture-sensitive-error' "$TEST_ROOT/last.log"
[[ $(state current) == "$A" && $(state pending) == "$C" && $(count api-up) == "$api_count" ]]
original_backup=$(state backup); backup_count=$(count backup)
unset FAKE_FAIL
expect_fail "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER" --release "$B"
invoke "$ROOT/deploy/update-backend.sh" update --deploy-dir "$SERVER" --release "$C"
[[ $(state current) == "$C" && $(state backup) == "$original_backup" && $(count backup) == "$backup_count" ]]
passed 'Migration failure blocks switching versions and exact-release retry retains the backup'

stop_count=$(count stop)
expect_fail "$ROOT/deploy/update-backend.sh" rollback --deploy-dir "$SERVER"
[[ $(count stop) == "$stop_count" && -f $FAKE_ROOT/api-running ]]
passed 'Rollback across changed migrations is refused before stopping the live API'

compose_sha=$(kv "$FAKE_OSS/latest-linux-amd64.tsv" compose_sha)
cp "$FAKE_OSS/blobs/$compose_sha.txt" "$TEST_ROOT/good-compose"
printf 'corrupt' >> "$FAKE_OSS/blobs/$compose_sha.txt"
expect_fail "$ROOT/deploy/update-backend.sh" init --deploy-dir "$TEST_ROOT/corrupt-server"
[[ ! -e $TEST_ROOT/corrupt-server/.env ]]
cp "$TEST_ROOT/good-compose" "$FAKE_OSS/blobs/$compose_sha.txt"
passed 'Corrupt OSS payloads fail checksum verification before initialization'

printf 'another source\n' >> "$FIXTURE/apps/main.rs"
invoke "$FIXTURE/upload-server.sh" --build-only
D=$(kv "$FIXTURE/target/offline-releases/last-linux-amd64.tsv" release)
export FAKE_FAIL=upload-manifest
expect_fail "$FIXTURE/upload-server.sh" --publish-release "$D"
[[ $(kv "$FAKE_OSS/latest-linux-amd64.tsv" release) == "$C" ]]
unset FAKE_FAIL
passed 'Incomplete upload never promotes the latest version'

export FAKE_SOURCE_RACE="$FIXTURE/apps/main.rs"
builds=$(count build)
expect_fail "$FIXTURE/upload-server.sh" --build-only
[[ $(count build) == "$builds" ]]
unset FAKE_SOURCE_RACE
passed 'Concurrent source editing aborts before image construction'

[[ ! -f $FAKE_ROOT/forbidden-interpreter ]]
passed 'No Python or jq executable was invoked by either deployment path'
printf '%s shell integration checks passed. No real containers or network requests were used.\n' "$PASS"
