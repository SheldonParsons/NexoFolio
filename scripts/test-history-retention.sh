#!/usr/bin/env bash
# Creates and destroys ONLY this process's private cluster; never sources .env.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="${PG_BIN:-/opt/homebrew/opt/postgresql@17/bin}:$PATH"
retention_out="${RETENTION_EVIDENCE_DIR:-$(pwd)/docs/audits/2026-10-14-history-retention}"
mkdir -p "$retention_out"
retention_tmp=$(mktemp -d /tmp/nexofolio-step09.XXXXXX)
retention_port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
cleanup() {
  pg_ctl -D "$retention_tmp/pg" stop -m fast >/dev/null 2>&1 || true
  cp "$retention_tmp/pg.log" "$retention_out/postgres.log"
  rm -rf "$retention_tmp"
}
trap cleanup EXIT
initdb -D "$retention_tmp/pg" -A trust --no-locale >/dev/null
pg_ctl -D "$retention_tmp/pg" -l "$retention_tmp/pg.log" -o "-h 127.0.0.1 -p $retention_port -k $retention_tmp" start >/dev/null
retention_cases=(boundaries_references_and_real_knowledge_reads protected_states_and_bounded_history real_transactions_fence_references_and_rollback_interruption)
if [ "$#" -gt 0 ]; then retention_cases=("$@"); fi
for retention_case in "${retention_cases[@]}"; do
  if [[ "$retention_case" != *::* ]]; then retention_case="retention::$retention_case"; fi
  retention_database="nexofolio_recovery_retention"
  createdb -h 127.0.0.1 -p "$retention_port" "$retention_database"
  TEST_DATABASE_URL="postgres://$(id -un)@127.0.0.1:$retention_port/$retention_database" \
    cargo test -p nexofolio-backend --test curation_queue "$retention_case" -- --ignored --exact --nocapture
  dropdb -h 127.0.0.1 -p "$retention_port" "$retention_database"
done
