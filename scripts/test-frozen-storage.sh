#!/usr/bin/env bash
# Only this private PostgreSQL; no user database or network upstream.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="${PG_BIN:-/opt/homebrew/opt/postgresql@17/bin}:$PATH"
frozen_tmp=$(mktemp -d /tmp/nexofolio-step04/storage.XXXXXX)
frozen_port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
cleanup() { pg_ctl -D "$frozen_tmp/pg" stop -m fast >/dev/null 2>&1 || true; rm -rf "$frozen_tmp"; }
trap cleanup EXIT
initdb -D "$frozen_tmp/pg" -A trust --no-locale >/dev/null
pg_ctl -D "$frozen_tmp/pg" -l "$frozen_tmp/pg.log" -o "-h 127.0.0.1 -p $frozen_port -k $frozen_tmp" start >/dev/null
frozen_index=0
for frozen_case in chunks_fence_owners_publish_atomically_and_reset_only_their_project additive_migration_preserves_old_input_archive_and_batches; do
  frozen_index=$((frozen_index+1))
  frozen_database="nexofolio_recovery_frozen_$frozen_index"
  createdb -h 127.0.0.1 -p "$frozen_port" "$frozen_database"
  TEST_DATABASE_URL="postgres://$(id -un)@127.0.0.1:$frozen_port/$frozen_database" cargo test -p nexofolio-backend --test curation_queue "frozen::$frozen_case" -- --ignored --exact --nocapture
done
