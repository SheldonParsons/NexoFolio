#!/usr/bin/env bash
# Private PostgreSQL + synthetic upstream only; no configured app services or paid models.
set -euo pipefail
cd "$(dirname "$0")/.."
queue_pg_bin="${PG_BIN:-}"
if [[ -z "$queue_pg_bin" && -x /opt/homebrew/opt/postgresql@17/bin/initdb ]]; then
  queue_pg_bin=/opt/homebrew/opt/postgresql@17/bin
fi
if [[ -n "$queue_pg_bin" ]]; then export PATH="$queue_pg_bin:$PATH"; fi
queue_tmp=$(mktemp -d "${TMPDIR:-/tmp}/nexofolio-queue.XXXXXX")
queue_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
cleanup() {
  pg_ctl -D "$queue_tmp/data" stop -m fast >/dev/null 2>&1 || true
  rm -rf "$queue_tmp"
}
trap cleanup EXIT
initdb -D "$queue_tmp/data" -A trust --no-locale > "$queue_tmp/init.log"
pg_ctl -D "$queue_tmp/data" -l "$queue_tmp/server.log" -o "-h 127.0.0.1 -p $queue_port -k $queue_tmp" start >/dev/null
queue_url="postgres://$(id -un)@127.0.0.1:$queue_port"
for queue_database in nexofolio_recovery_append nexofolio_recovery_upgrade nexofolio_recovery_queue nexofolio_recovery_process nexofolio_recovery_children nexofolio_recovery_audit nexofolio_access_test_queue; do
  createdb -h 127.0.0.1 -p "$queue_port" "$queue_database"
done
run_one() {
  local queue_db="$1" queue_test="$2" queue_log="$queue_tmp/$1.log"
  TEST_DATABASE_URL="$queue_url/$queue_db" cargo test -p nexofolio-backend --test curation_queue "$queue_test" -- --ignored --exact | tee "$queue_log"
  rg -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$queue_log"
}
run_one nexofolio_recovery_append append::append_orders_fences_and_rolls_back_atomically
run_one nexofolio_recovery_upgrade migration::migration_adopts_existing_running_candidate_without_rewriting_evidence
run_one nexofolio_recovery_queue admission_fairness_fencing_and_decisions
run_one nexofolio_recovery_children children::parent_closure_interrupts_child_and_releases_next_scope
TEST_DATABASE_URL="$queue_url/nexofolio_recovery_audit" cargo test -p nexofolio-backend --test curation_queue_audit -- --ignored --test-threads=1
TEST_DATABASE_URL="$queue_url/nexofolio_recovery_process" cargo test -p nexofolio-backend --test curation_restart -- --ignored
TEST_DATABASE_URL="$queue_url/nexofolio_access_test_queue" cargo test -p nexofolio-backend \
  --test candidate_review --test candidate_operations --test document_lifecycle \
  --test field_values --test project_management --test navigation_mcp -- --ignored
