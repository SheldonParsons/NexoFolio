#!/usr/bin/env python3
"""Actual v2/v3 process restarts and corruption/preparation failures in owned PostgreSQL."""

import importlib.util, pathlib, argparse, json, time, hashlib, copy, urllib.parse

spec = importlib.util.spec_from_file_location(
    "frozen", pathlib.Path(__file__).with_name("test-frozen-input.py")
)
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)


def wait(fn, timeout=60):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = fn()
        if result:
            return result
        time.sleep(0.1)
    raise TimeoutError("condition did not settle")


def expire(h, db):
    h.sql(
        db,
        "UPDATE knowledge.curation_tasks SET lease_until=now()-interval '1 second' WHERE state='running'; UPDATE knowledge.candidates SET updated_at=now()-interval '121 seconds' WHERE state='running';",
    )


def state(h, db):
    raw = h.sql(
        db,
        "SELECT jsonb_build_object('task',t.id,'candidate',c.id,'state',t.state,'run',t.run_id,'generation',t.generation,'input',c.execution_input,'batches',c.batches,'progress',c.progress) FROM knowledge.curation_tasks t JOIN knowledge.candidates c ON c.id=t.candidate_id ORDER BY t.created_at DESC LIMIT 1",
    )
    return json.loads(raw) if raw else None


from frozen_input_fixture import rich_seed


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--baseline", required=True)
    p.add_argument("--optimized", required=True)
    p.add_argument("--admin", required=True)
    p.add_argument("--out", required=True)
    a = p.parse_args()
    out = pathlib.Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    h = f.Harness(out, pathlib.Path(a.admin).resolve())
    ledger = []
    try:
        seed = "nexofolio_step04_rich_seed"
        h.create(seed)
        h.start(seed, a.optimized, "rich-seed")
        project = rich_seed(h, seed)
        sources = json.loads(h.sql(seed, "SELECT jsonb_agg(DISTINCT platform) FROM intake.batches"))
        assert sources == ["nexofolio-frozen-synthetic"]
        assert h.sql(seed, "SELECT count(*) FROM intake.fetcher_release_policy") == "0"
        (out / "network-boundary.json").write_text(json.dumps({"platforms": sources, "fetcher_version_header": False, "fetcher_policy_rows": 0, "model_identity_base": h.up}) + "\n")
        h.stop()
        equality = None
        first_prompt = None
        logical_calls = None
        for mode, binary in [("v2", a.baseline), ("v3", a.optimized)]:
            db = "nexofolio_step04_" + mode
            h.create(db, seed)
            result = h.measure(db, binary, mode, project)
            frozen = h.evidence(db, result["candidate"])
            assert frozen["relations"] > 0 and len(frozen["relation_environments"]) == 2
            (out / (mode + "-equivalence.json")).write_text(
                json.dumps(frozen, indent=2) + "\n"
            )
            if equality is None:
                equality = frozen
                first_prompt = result["model_calls"][0]["sha256"]
            else:
                assert equality == frozen
                assert first_prompt == result["model_calls"][0]["sha256"]
            saved = state(h, db)
            input_hash = f.digest(saved["input"])
            assert saved["input"]["version"] == (2 if mode == "v2" else 3)
            # The process is gone. Rotate its lease in the private DB to exercise actual takeover promptly.
            expire(h, db)
            f.Upstream.calls = []
            f.Upstream.mode = "ok"
            h.start(db, a.optimized, mode + "-resumed")
            final = wait(
                lambda: (
                    v
                    if (v := state(h, db)) and v["state"] in ["done", "needs_attention"]
                    else None
                ),
                180,
            )
            assert final["state"] == "done", final["progress"]
            assert final["candidate"] == saved["candidate"]
            assert final["task"] == saved["task"]
            assert f.digest(final["input"]) == input_hash
            assert final["generation"] > saved["generation"]
            assert len(final["batches"]) >= len(saved["batches"])
            for batch in saved["batches"]:
                after = next(b for b in final["batches"] if b["id"] == batch["id"])
                assert after["commands"] == batch["commands"]
            assert any(b["commands"] for b in saved["batches"])
            description_calls = sum(c["stage"] == "describe" for c in f.Upstream.calls)
            assert (
                description_calls
                == len(frozen["endpoints"]) - saved["progress"]["generated"]
            )
            calls = sorted(c["sha256"] for c in f.Upstream.calls)
            if logical_calls is None:
                logical_calls = calls
            else:
                assert (
                    calls == logical_calls
                ), "actual description/field/directory model payloads differ"
            h.stop()
            # New process, raw values endpoint: arbitrary precision and environment ownership survive.
            h.start(db, a.optimized, mode + "-values")
            endpoints = json.loads(
                h.sql(
                    db,
                    f"SELECT jsonb_agg(jsonb_build_object('id',id,'path',path_template)) FROM observe.endpoints WHERE project_id='{project}'",
                )
            )
            eid = next(e["id"] for e in endpoints if e["path"] == "/widgets/detail")
            query = urllib.parse.urlencode(
                {
                    "archive": saved["candidate"],
                    "field": "response_body:200:unknownBusinessCode",
                }
            )
            status, values = h.request(
                f"/v1/projects/{project}/endpoints/{eid}/field-values?" + query
            )
            assert status == 200, (status, values)
            assert values["summary"]["total"] == 1
            query += "&value_id=" + urllib.parse.quote(values["items"][0]["id"])
            status, value = h.request(
                f"/v1/projects/{project}/endpoints/{eid}/field-values?" + query
            )
            assert status == 200 and value["json"] == "1.234567890123456789", (
                status,
                value,
            )
            h.stop()
            ledger.append(
                {
                    "case": mode + " actual API restart",
                    "same_input_sha256": input_hash,
                    "result": "passed",
                    "generation_before": saved["generation"],
                    "generation_after": final["generation"],
                    "raw_decimal_exact": True,
                    "task_id": saved["task"],
                    "candidate_id": saved["candidate"],
                    "saved_descriptions_not_repeated": saved["progress"]["generated"],
                    "description_calls_after_restart": description_calls,
                }
            )
        # Each corruption uses a new database cloned from a complete, actually written v3 record.
        source = "nexofolio_step04_v3"
        saved = state(h, source)
        manifest = saved["input"]["manifest"]
        ref = next(iter(manifest["endpoints"].values()))["inputs"][0]["content"][
            "sha256"
        ]
        for fault in ["missing", "hash", "manifest", "snapshot", "scope"]:
            db = "nexofolio_step04_fault_" + fault
            h.create(db, source)
            if fault == "missing":
                h.sql(
                    db,
                    f"DELETE FROM knowledge.frozen_input_chunks WHERE sha256='{ref}'",
                )
            elif fault == "hash":
                h.sql(
                    db,
                    f"UPDATE knowledge.frozen_input_chunks SET content=content||' ' WHERE sha256='{ref}'",
                )
            elif fault == "manifest":
                h.sql(
                    db,
                    "UPDATE knowledge.candidates SET execution_input=jsonb_set(execution_input,'{manifest_hash}','\"invalid\"')",
                )
            elif fault == "snapshot":
                h.sql(
                    db,
                    "UPDATE knowledge.candidates SET execution_input=jsonb_set(execution_input,'{manifest,snapshot}','\"00000000-0000-0000-0000-000000000000\"')",
                )
            else:
                h.sql(
                    db,
                    "UPDATE knowledge.candidates SET execution_input=jsonb_set(execution_input,'{summaries,0,path_template}','\"/wrong-scope\"')",
                )
            h.sql(
                db,
                "UPDATE knowledge.candidates SET state='partial'; UPDATE knowledge.curation_tasks SET state='needs_attention';",
            )
            f.Upstream.calls = []
            f.Upstream.mode = "ok"
            h.start(db, a.optimized, "fault-" + fault)
            status, _ = h.request(
                f'/v1/projects/{project}/curate/candidates/{saved["candidate"]}/resume',
                {},
            )
            assert status == 202
            rejected = wait(
                lambda: (
                    v
                    if (v := state(h, db)) and v["state"] == "needs_attention"
                    else None
                )
            )
            time.sleep(1)
            assert not f.Upstream.calls
            status, _ = h.request(
                f'/v1/projects/{project}/curate/candidates/{saved["candidate"]}/apply',
                {},
            )
            assert status >= 400
            assert f.digest(rejected["batches"]) == f.digest(saved["batches"])
            h.stop()
            ledger.append(
                {
                    "case": fault,
                    "result": "blocked before model and apply",
                    "model_calls": 0,
                }
            )
        # Interrupt before publication; recover the same task/candidate using an entirely new attempt.
        db = "nexofolio_step04_preparation"
        h.create(db)
        h.start(db, a.optimized, "preparation-seed")
        meta = h.seed(db, 30)
        project2 = meta["project"]
        f.Upstream.calls = []
        f.Upstream.mode = "hold"
        assert h.request("/v1/projects/" + project2 + "/curate", {})[0] == 202
        wait(
            lambda: int(h.sql(db, "SELECT count(*) FROM knowledge.frozen_input_chunks"))
            > 3
        )
        interrupted = state(h, db)
        assert interrupted["input"] is None
        h.api.kill()
        h.api.wait()
        h.stop()
        old_snapshots = h.sql(
            db, "SELECT DISTINCT snapshot_id FROM knowledge.frozen_input_chunks"
        ).splitlines()
        expire(h, db)
        h.start(db, a.optimized, "preparation-resumed")
        wait(lambda: len(f.Upstream.calls) > 0, 180)
        restored = state(h, db)
        assert restored["candidate"] == interrupted["candidate"]
        assert restored["task"] == interrupted["task"]
        assert restored["input"]["manifest"]["snapshot"] not in old_snapshots
        assert (
            h.request(
                "/v1/projects/"
                + project2
                + "/curate/tasks/"
                + h.sql(
                    db,
                    "SELECT id FROM knowledge.curation_tasks ORDER BY created_at DESC LIMIT 1",
                )
                + "/stop",
                {},
            )[0]
            == 204
        )
        h.stop()
        f.Upstream.calls = []
        f.Upstream.mode = "ok"
        h.start(db, a.optimized, "stopped-restarted")
        time.sleep(2)
        assert not f.Upstream.calls
        assert state(h, db)["state"] == "stopped"
        h.stop()
        ledger.append(
            {
                "case": "preparation interruption and stopped restart",
                "result": "passed",
                "abandoned_attempts": len(old_snapshots),
                "same_candidate": True,
            }
        )
        (out / "results.json").write_text(
            json.dumps(
                {
                    "ledger": ledger,
                    "full_input_equivalence": True,
                    "actual_first_model_request_equal": True,
                    "all_resumed_model_payloads_equal": True,
                    "cross_interface_relations": equality["relations"],
                    "relation_environments": equality["relation_environments"],
                    "lease_expiry": "injected in private DB; all recovery performed by newly started API",
                    "paid_calls": 0,
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n"
        )
    finally:
        h.cleanup()
        (out / "cleanup.json").write_text(
            json.dumps(
                {
                    "private_pg_deleted": True,
                    "owned_processes_stopped": True,
                    "completed_cases": ledger,
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n"
        )


if __name__ == "__main__":
    main()
