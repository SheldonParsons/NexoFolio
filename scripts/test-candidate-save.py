#!/usr/bin/env python3
"""Step-05 assessment. Private real API/PG; existing seed and localhost model only."""
import argparse
import hashlib
import json
import math
import os
import pathlib
import subprocess
import threading
import time
from urllib.parse import urlencode

from frozen_input_harness import Harness, PG, Upstream, digest, encoded, run


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


class Bench(Harness):
    probe_dir = None
    samples = 40
    detail = True
    fixed_commands = None
    history_multiplier = 1

    def request(self, path, body=None):
        if path == "/v1/collect/batches":
            body = dict(body, platform="nexofolio-step05-synthetic")
        return super().request(path, body)

    def env(self, db, api_port):
        env = super().env(db, api_port)
        if self.probe_dir:
            env.update(SAVE_PROBE_DIR=str(self.probe_dir), SAVE_PROBE_REPEATS=str(self.samples))
            if self.detail:
                env["SAVE_PROBE_DETAIL"] = "1"
        return env

    def configure(self):
        run([PG / "pg_ctl", "-D", self.tmp / "pg", "stop", "-m", "fast"], stdout=subprocess.DEVNULL)
        with (self.tmp / "pg/postgresql.conf").open("a") as f:
            f.write("\nshared_preload_libraries='pg_stat_statements'\ntrack_io_timing=on\ntrack_wal_io_timing=on\npg_stat_statements.track='all'\n")
        run([PG / "pg_ctl", "-D", self.tmp / "pg", "-l", self.tmp / "pg.log", "-o", f"-h 127.0.0.1 -p {self.pg_port} -k {self.tmp}", "start"], stdout=subprocess.DEVNULL)
        self.pg_pid = int((self.tmp / "pg/postmaster.pid").read_text().splitlines()[0])

    def wait_file(self, path, seconds=600):
        until = time.monotonic() + seconds
        while not path.exists():
            assert self.api.poll() is None, "API exited"
            if time.monotonic() > until:
                raise TimeoutError(str(path))
            time.sleep(0.05)

    def snapshot(self, db):
        return json.loads(self.sql(db, """SELECT jsonb_build_object(
          'db',(SELECT to_jsonb(d) FROM pg_stat_database d WHERE datname=current_database()),
          'wal',(SELECT to_jsonb(w) FROM pg_stat_wal w),
          'lsn',pg_current_wal_insert_lsn()::text,
          'statements',(SELECT jsonb_agg(to_jsonb(s)) FROM pg_stat_statements s WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND query NOT LIKE '%pg_stat_%'),
          'relations',(SELECT jsonb_agg(jsonb_build_object('name',c.oid::regclass::text,'heap',pg_relation_size(c.oid),'total',pg_total_relation_size(c.oid),'toast',CASE WHEN c.reltoastrelid=0 THEN 0 ELSE pg_total_relation_size(c.reltoastrelid) END)) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='knowledge' AND c.relname IN ('candidates','frozen_input_chunks','curation_tasks')),
          'connections',(SELECT count(*) FROM pg_stat_activity WHERE datname=current_database()))"""))

    def candidate(self, db):
        return json.loads(self.sql(db, """SELECT jsonb_build_object('id',id,'base_hash',base_hash,'base_content_hash',md5(base::text),'state',state,'base_bytes',octet_length(base::text),'view_bytes',octet_length(view::text),'batches_bytes',octet_length(batches::text),'base_fields',jsonb_array_length(base->'fields'),'view_fields',jsonb_array_length(view->'fields'),'groups',jsonb_array_length(batches),'commands',(SELECT sum(jsonb_array_length(b->'commands')) FROM jsonb_array_elements(batches) b),'content_hash',md5(view::text||batches::text),'input_hash',md5(execution_input::text),'chunks_hash',(SELECT md5(string_agg(sha256||content,'' ORDER BY snapshot_id,sha256)) FROM knowledge.frozen_input_chunks),'chunks',(SELECT count(*) FROM knowledge.frozen_input_chunks),'input_fields',(SELECT sum(jsonb_array_length(b->'keys')) FROM jsonb_each(execution_input->'manifest'->'endpoints') e CROSS JOIN LATERAL jsonb_array_elements(e.value->'inputs') b)) FROM knowledge.candidates ORDER BY created_at DESC LIMIT 1"""))


def measure(h, db, binary, label, project, samples, detail):
    h.probe_dir = h.tmp / label
    h.probe_dir.mkdir()
    h.samples, h.detail = samples, detail
    Upstream.mode, Upstream.calls = "run", []
    pid = h.start(db, binary, label)
    target = h.sql(db, "SELECT id FROM observe.endpoints WHERE path_template='/synthetic/orders'")
    started = time.monotonic()
    status, task = h.request(f"/v1/projects/{project}/curate/describe/{target}", {})
    assert status == 202, (status, task)
    h.wait_file(h.probe_dir / "ready.json")
    preparation_seconds = time.monotonic() - started
    commands = json.loads((h.probe_dir / "ready.json").read_text())
    if h.history_multiplier > 1:
        # Repeat only already-committed idempotent puts; the existing view is unchanged.
        h.sql(db, f"""UPDATE knowledge.candidates SET batches=(
          SELECT jsonb_agg(CASE WHEN jsonb_array_length(b->'commands')=0 THEN b ELSE
            jsonb_set(b,'{{commands}}',(SELECT jsonb_agg(cmd ORDER BY n,ord)
            FROM generate_series(1,{h.history_multiplier}) n
            CROSS JOIN LATERAL jsonb_array_elements(b->'commands') WITH ORDINALITY AS a(cmd,ord))) END ORDER BY batch_ord)
          FROM jsonb_array_elements(batches) WITH ORDINALITY AS x(b,batch_ord))""")
    initial = h.candidate(db)
    original_batches = json.loads(h.sql(db, "SELECT batches FROM knowledge.candidates ORDER BY created_at DESC LIMIT 1"))
    if h.fixed_commands is None:
        h.fixed_commands = encoded(commands).replace(initial["id"], "CANDIDATE")
    commands = json.loads(h.fixed_commands.replace("CANDIDATE", initial["id"]))
    save(h.out / (label + "-commands.json"), commands)
    save(h.probe_dir / "commands.json", commands)
    h.sql(db, "SELECT pg_stat_statements_reset()")
    before = h.snapshot(db)
    host_before = {"load": os.getloadavg(), "vm_stat": subprocess.check_output(["vm_stat"], text=True), "top": subprocess.check_output(["top", "-l", "1", "-n", "0"], text=True)}
    resources, waits = [], []
    stop = threading.Event()
    started = time.monotonic()

    def sample():
        while not stop.is_set():
            stamp = time.monotonic()
            rss = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)]).strip()) * 1024
            resources.append({"seconds": stamp - started, "api_rss_bytes": rss})
            stop.wait(max(0, 0.1 - (time.monotonic() - stamp)))

    def activity():
        while not stop.is_set():
            result = h.sql(db, "SELECT coalesce(jsonb_agg(jsonb_build_object('pid',pid,'state',state,'wait_type',wait_event_type,'wait',wait_event,'query',left(query,220))),'[]') FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()")
            waits.append({"seconds": time.monotonic() - started, "activity": json.loads(result)})
            stop.wait(0.25)

    threads = [threading.Thread(target=sample), threading.Thread(target=activity)]
    for thread in threads:
        thread.start()
    try:
        time.sleep(0.3)
        (h.probe_dir / "go").touch()
        h.wait_file(h.probe_dir / "done", 900)
    finally:
        stop.set()
        for thread in threads:
            thread.join()
    wall = time.monotonic() - started
    time.sleep(1.1)  # PostgreSQL cumulative statistics flush; outside measured window.
    after = h.snapshot(db)
    final = h.candidate(db)
    assert final["commands"] == initial["commands"] + samples * len(commands), (initial, final)
    assert final["input_hash"] == initial["input_hash"] and final["base_hash"] == initial["base_hash"] and final["base_content_hash"] == initial["base_content_hash"] and final["chunks_hash"] == initial["chunks_hash"]
    persisted_batches = json.loads(h.sql(db, "SELECT batches FROM knowledge.candidates ORDER BY created_at DESC LIMIT 1"))
    assert len(original_batches) == len(persisted_batches)
    appended = 0
    for old, new in zip(original_batches, persisted_batches):
        assert {k: v for k, v in old.items() if k != "commands"} == {k: v for k, v in new.items() if k != "commands"}
        assert new["commands"][:len(old["commands"])] == old["commands"]
        extra = new["commands"][len(old["commands"]):]
        if extra:
            assert extra == commands * samples
            appended += 1
    assert appended == 1
    fields = json.loads(h.sql(db, "SELECT view->'fields' FROM knowledge.candidates ORDER BY created_at DESC LIMIT 1"))
    checked = []
    sources = []
    archive_values = []
    for command in commands:
        assert command["do"] == "put_field_note", command.keys()
        actual = next(f for f in fields if f["endpoint"] == command["endpoint"] and f["path"] == command["path"])
        for key in ["text", "values", "context"]:
            default = [] if key == "values" else None
            assert actual.get(key, default) == command.get(key, default), (key, command["path"])
        checked.append(command["path"])
        query = urlencode({"archive": final["id"], "field": command["path"]})
        status, value = h.request(f"/v1/projects/{project}/endpoints/{target}/field-values?{query}")
        assert status == 200, (command["path"], status, value)
        assert value["summary"] == command["context"]["observations"]
        expected = {"response_header:200:content-type": "application/json", "request_body:orderId": 101,
                    "request_body:remark": "合成测试备注", "response_body:200:code": 200,
                    "response_body:200:data": {"orderId": 101, "status": "created"},
                    "response_body:200:data.orderId": 101, "response_body:200:data.status": "created"}
        assert len(value["items"]) == 1 and not value["has_more"]
        item = value["items"][0]
        assert item["sources"] and all(s["endpoint"] == target for s in item["sources"])
        full_query = urlencode({"archive": final["id"], "field": command["path"], "value_id": item["id"]})
        status, full = h.request(f"/v1/projects/{project}/endpoints/{target}/field-values?{full_query}")
        assert status == 200 and json.loads(full["json"]) == expected[command["path"]]
        archive_values.append({"field": command["path"], "page": value, "full_value": full})
        sources.append({"path": command["path"], "status": status, "response_sha256": digest(value)})
    save(h.out / f"{label}-archive-values.json", archive_values)
    h.sql(db, "UPDATE knowledge.curation_tasks SET run_id=gen_random_uuid(),generation=generation+1 WHERE state='running'")
    (h.probe_dir / "reject").touch()
    h.wait_file(h.probe_dir / "rejected.json", 30)
    assert json.loads((h.probe_dir / "rejected.json").read_text()) is True
    rejected = h.candidate(db)
    assert rejected["content_hash"] == final["content_hash"], "stale writer changed content"
    log = (h.out / (label + "-api.log")).read_text()
    ledger = [json.loads(line.split("SAVE_SAMPLE ", 1)[1]) for line in log.splitlines() if "SAVE_SAMPLE " in line]
    probes = [json.loads(line.split("SAVE_PROBE ", 1)[1]) for line in log.splitlines() if "SAVE_PROBE " in line]
    assert len(ledger) == samples
    for row in ledger:
        row.pop("command_json", None)
    normalized = json.loads(encoded(commands).replace(initial["id"], "CANDIDATE"))
    result = {"label": label, "binary_sha256": h.active_binary_sha, "detail": detail, "n": samples, "command_count": len(commands), "command_sha256": digest(commands), "normalized_command_sha256": digest(normalized), "preparation_to_field_ready_seconds": preparation_seconds, "save_window_seconds": wall, "initial": initial, "final": final, "checked_fields": checked, "stale_owner_rejected": True, "stale_write_content_unchanged": True, "rss_peak_bytes": max(r["api_rss_bytes"] for r in resources), "rss_before_bytes": resources[0]["api_rss_bytes"], "resource_max_gap_ms": max((b["seconds"]-a["seconds"])*1000 for a,b in zip(resources,resources[1:]))}
    result["round_order_history_prefix_and_exact_appends"] = True
    result["candidate_mapping"] = {"placeholder": "CANDIDATE", "actual": initial["id"], "normalization": "candidate archive UUID only; all other Command content retained"}
    result["archive_reads"] = sources
    result["host_before"] = host_before
    result["host_after_load"] = os.getloadavg()
    result["localhost_model_calls"] = list(Upstream.calls)
    result["synthetic_history_multiplier"] = h.history_multiplier
    for suffix, value in [("result", result), ("ledger", ledger), ("probes", probes), ("resources", resources), ("waits", waits), ("sql-before", before), ("sql-after", after)]:
        save(h.out / f"{label}-{suffix}.json", value)
    h.stop()
    h.probe_dir = None
    print(label, len(commands), "fields", "median_ms", sorted(x["apply_ms"] for x in ledger)[len(ledger)//2], "rss_MiB", result["rss_peak_bytes"]/2**20, flush=True)
    return result


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--api", required=True)
    p.add_argument("--comparison-api")
    p.add_argument("--comparison-no-detail", action="store_true")
    p.add_argument("--admin", required=True)
    p.add_argument("--out", type=pathlib.Path, required=True)
    p.add_argument("--sizes", default="3,101,501")
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--samples", type=int, default=40)
    p.add_argument("--no-detail", action="store_true")
    p.add_argument("--history-multiplier", type=int, default=1)
    p.add_argument("--include-history", action="store_true", help="Also test 4000 historical commands from the same seed and fixed command")
    args = p.parse_args()
    assert args.samples >= 2 and args.repeats >= 1 and args.history_multiplier >= 1
    assert not (args.out / "results.json").exists(), "Use a new evidence directory"
    args.out.mkdir(parents=True, exist_ok=True)
    pathlib.Path("/tmp/nexofolio-step04").mkdir(exist_ok=True)
    h = Bench(args.out, pathlib.Path(args.admin).resolve())
    h.history_multiplier = args.history_multiplier
    results = []
    try:
        h.configure()
        sizes = [int(n) for n in args.sizes.split(",")]
        assert sizes == sorted(set(sizes)) and min(sizes) >= 3
        seed_db = "nexofolio_step04_save_seed"
        h.create(seed_db)
        h.sql(seed_db, "CREATE EXTENSION pg_stat_statements")
        save(args.out / "postgres-settings.json", json.loads(h.sql(seed_db, "SELECT jsonb_object_agg(name,setting) FROM pg_settings WHERE name IN ('server_version','shared_buffers','work_mem','max_connections','synchronous_commit','wal_compression','shared_preload_libraries','track_io_timing','track_wal_io_timing','block_size','fsync','full_page_writes')")))
        h.start(seed_db, args.api, "seed")
        meta = h.seed(seed_db, max(sizes))
        h.stop()
        meta["template_rows"] = json.loads(h.sql(seed_db, "SELECT jsonb_build_object('endpoints',(SELECT md5(jsonb_agg(to_jsonb(e) ORDER BY id)::text) FROM observe.endpoints e),'fields',(SELECT md5(jsonb_agg(to_jsonb(f) ORDER BY endpoint_id,path)::text) FROM knowledge.field_notes f))"))
        save(args.out / "fixture.json", meta)
        assert h.sql(seed_db, "SELECT count(*) FROM intake.fetcher_release_policy") == "0"
        sources = json.loads(h.sql(seed_db, "SELECT jsonb_agg(DISTINCT platform) FROM intake.batches"))
        assert sources == ["nexofolio-step05-synthetic"]
        save(args.out / "network-boundary.json", {"producer": "nexofolio-step05-synthetic", "fetcher_policy_rows": 0, "persisted_platforms": sources, "fetcher_version_header": False, "model_and_identity_base": h.up, "business_requests": 0, "paid_calls": 0})
        for size in sizes:
            template = f"nexofolio_step04_save_t{size}"
            h.create(template, seed_db)
            h.sql(template, f"""CREATE TEMP TABLE removed AS SELECT id FROM observe.endpoints WHERE id NOT IN (SELECT id FROM observe.endpoints ORDER BY (path_template='/synthetic/orders') DESC,path_template LIMIT {size});
            DELETE FROM observe.fingerprints WHERE endpoint_id IN (SELECT id FROM removed);
            DELETE FROM observe.declarations WHERE endpoint_id IN (SELECT id FROM removed);
            DELETE FROM knowledge.field_notes WHERE endpoint_id IN (SELECT id FROM removed);
            DELETE FROM observe.endpoints WHERE id IN (SELECT id FROM removed);""")
            for history in ([1, 1000] if args.include_history else [args.history_multiplier]):
                h.history_multiplier = history
                for repeat in range(args.repeats):
                    binaries = [("primary", args.api)]
                    if args.comparison_api:
                        binaries.append(("comparison", args.comparison_api))
                    for profile, binary in binaries:
                        label = f"{profile}-size{size}" + ("-history1000" if history == 1000 else "") + f"-r{repeat+1}"
                        db = "nexofolio_step04_save_" + label.replace("-", "_")
                        h.create(db, template)
                        print("starting", label, flush=True)
                        detail = not args.no_detail and not (profile == "comparison" and args.comparison_no_detail)
                        results.append(measure(h, db, pathlib.Path(binary).resolve(), label, meta["project"], args.samples, detail))
                        h.sql(db, "CHECKPOINT")
                        run([PG / "dropdb", "-h", "127.0.0.1", "-p", h.pg_port, db], stdout=subprocess.DEVNULL)
                        save(args.out / "results.json", results)
    finally:
        tmp, pg_pid = str(h.tmp), h.pg_pid
        ports = {"postgres": h.pg_port, "localhost_upstream": h.server.server_port}
        h.cleanup()
        save(args.out / "cleanup.json", {"owned_tmp": tmp, "owned_pg_pid": pg_pid, "ports": ports, "private_pg_deleted": not pathlib.Path(tmp).exists(), "api_stopped": h.api is None, "upstream_stopped": True, "paid_calls": 0})


if __name__ == "__main__":
    main()
