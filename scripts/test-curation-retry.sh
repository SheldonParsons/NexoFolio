#!/usr/bin/env bash
# Isolated PostgreSQL + actual API processes + fixed localhost model/identity upstreams.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -n "${1:-}" && "${1:-}" != --two-hour ]]; then echo "Usage: $0 [--two-hour]" >&2; exit 2; fi
retry_pg_bin="${PG_BIN:-/opt/homebrew/opt/postgresql@17/bin}"
export PATH="$retry_pg_bin:$PATH"
mkdir -p /tmp/nexofolio-step02
retry_tmp=$(mktemp -d /tmp/nexofolio-step02/run.XXXXXX)
retry_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
export RETRY_EVIDENCE_DIR="$retry_tmp/evidence"
cleanup() {
  pg_ctl -D "$retry_tmp/pg" stop -m fast > /dev/null 2>&1 || true
  rm -rf "$retry_tmp/pg"
  # Fixtures pin their API binary so rebuilding cannot silently change a running test.
  find "$retry_tmp/evidence" -type f -name api-runtime -delete 2>/dev/null || true
  echo "Evidence retained at $retry_tmp"
}
trap cleanup EXIT
initdb -D "$retry_tmp/pg" -A trust --no-locale > "$retry_tmp/init.log"
pg_ctl -D "$retry_tmp/pg" -l "$retry_tmp/postgres.log" -o "-h 127.0.0.1 -p $retry_port -k $retry_tmp" start > /dev/null
retry_url="postgres://$(id -un)@127.0.0.1:$retry_port"
run_test() {
  local database="nexofolio_recovery_$1" suite="$2" test="$3"
  createdb -h 127.0.0.1 -p "$retry_port" "$database"
  TEST_DATABASE_URL="$retry_url/$database" cargo test -p nexofolio-backend --test "$suite" "$test" -- --ignored --exact --nocapture > "$retry_tmp/$1.log" 2>&1 || { cat "$retry_tmp/$1.log"; return 1; }
  rg -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$retry_tmp/$1.log"
  tail -3 "$retry_tmp/$1.log"
}
run_test step02_scope curation_queue retry::retry_states_keep_scope_order_owner_and_parent_closure
run_test step02_upgrade curation_queue retry::first_step_tasks_upgrade_without_rewriting_identity_or_history
run_test step02_smoke curation_retry temporary_wait_releases_slot_and_restart_preserves_deadline
run_test step02_matrix curation_retry fault_attention_stop_and_partial_apply_matrix
run_test step02_database curation_retry database_outage_keeps_acknowledged_results_and_fences_old_owner
run_test step02_stages curation_retry description_and_directory_use_same_durable_recovery
run_test step02_warm curation_retry shared_cooldown_does_not_starve_a_healthy_project
run_test step02_diagnostics curation_retry diagnostics::current_task_diagnostics_and_exact_events_survive_restarts_and_decisions
if [[ "${1:-}" == --two-hour ]]; then
  run_test step02_long curation_retry two_hour_outage_automatically_resumes_original_candidate
elif [[ -n "${1:-}" ]]; then
  echo 'Usage: scripts/test-curation-retry.sh [--two-hour]' >&2
  exit 2
fi
