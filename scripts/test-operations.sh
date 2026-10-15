#!/usr/bin/env bash
# Isolated local model + identity + PostgreSQL. No configured services or paid model requests.
set -euo pipefail
cd "$(dirname "$0")/.."
operations_pg_bin="${PG_BIN:-/opt/homebrew/opt/postgresql@17/bin}"
export PATH="$operations_pg_bin:$PATH"
mkdir -p /tmp/nexofolio-operations
operations_tmp=$(mktemp -d /tmp/nexofolio-operations/run.XXXXXX)
operations_port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
cleanup() {
  pg_ctl -D "$operations_tmp/pg" stop -m fast >/dev/null 2>&1 || true
  rm -rf "$operations_tmp/pg"
  find "$operations_tmp" -type f -name api-runtime -delete
  echo "Evidence retained at $operations_tmp"
}
trap cleanup EXIT
initdb -D "$operations_tmp/pg" -A trust --no-locale > "$operations_tmp/init.log"
pg_ctl -D "$operations_tmp/pg" -l "$operations_tmp/postgres.log" -o "-h 127.0.0.1 -p $operations_port -k $operations_tmp" start >/dev/null
createdb -h 127.0.0.1 -p "$operations_port" nexofolio_operations_acceptance
TEST_DATABASE_URL="postgres://$(id -un)@127.0.0.1:$operations_port/nexofolio_operations_acceptance" OPERATIONS_EVIDENCE_DIR="$operations_tmp/evidence" cargo test -p nexofolio-backend --test operations real_process_metrics_permissions_tasks_and_diagnostics -- --ignored --exact --nocapture | tee "$operations_tmp/result.log"

createdb -h 127.0.0.1 -p "$operations_port" nexofolio_operations_projection
TEST_DATABASE_URL="postgres://$(id -un)@127.0.0.1:$operations_port/nexofolio_operations_projection" OPERATIONS_EVIDENCE_DIR="$operations_tmp/projection" cargo test -p nexofolio-backend --test operations projection::projection_upgrade_bounds_and_heartbeat_identity -- --ignored --exact --nocapture | tee "$operations_tmp/projection.log"
