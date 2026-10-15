"""Raw resource sampling and complete logical evidence comparison, outside application code."""

import threading, time, subprocess, json, hashlib, decimal
from frozen_input_harness import encoded, digest, Upstream


def measure(h, db, binary, label, project):
    Upstream.calls = []
    Upstream.mode = "first"
    Upstream.gate.clear()
    pid = h.start(db, binary, label)
    samples = []
    stop = threading.Event()
    h.sampler_stop = stop
    started = time.monotonic()

    def sample():
        while not stop.is_set():
            at = time.monotonic()
            result = subprocess.run(
                ["ps", "-axo", "pid=,ppid=,rss="],
                text=True,
                capture_output=True,
                check=True,
            )
            rows = [list(map(int, row.split())) for row in result.stdout.splitlines()]
            api = next((r[2] * 1024 for r in rows if r[0] == pid), 0)
            pg = sum(r[2] * 1024 for r in rows if r[0] == h.pg_pid or r[1] == h.pg_pid)
            samples.append(
                {
                    "seconds": at - started,
                    "api_rss_bytes": api,
                    "pg_process_rss_sum_bytes": pg,
                }
            )
            stop.wait(max(0, 0.1 - (time.monotonic() - at)))

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    time.sleep(0.3)
    before = h.stats(db)
    start = time.monotonic()
    status, task = h.request("/v1/projects/" + project + "/curate", {})
    assert status == 202, (status, task)
    deadline = time.monotonic() + 600
    while not Upstream.calls:
        if time.monotonic() > deadline:
            raise TimeoutError("first model " + label)
        if h.api.poll() is not None:
            raise RuntimeError("API died " + label)
        time.sleep(0.1)
    first = Upstream.calls[0]["at"]
    while True:
        saved = int(
            h.sql(
                db,
                "SELECT coalesce(max((progress->>'generated')::bigint),0) FROM knowledge.candidates",
            )
        )
        if saved >= 1:
            break
        if time.monotonic() > deadline:
            raise TimeoutError("first durable batch " + label)
        time.sleep(0.1)
    first_saved = time.monotonic()
    time.sleep(3)
    stop.set()
    sampler.join()
    after = h.stats(db)
    status, progress = h.request("/v1/projects/" + project + "/curate")
    assert status == 200
    result = {
        "label": label,
        "binary_sha256": h.active_binary_sha,
        "first_model_seconds": first - start,
        "first_saved_batch_seconds": first_saved - start,
        "saved_descriptions_at_boundary": saved,
        "observation_after_first_saved_seconds": 3,
        "api_peak_rss_bytes": max(r["api_rss_bytes"] for r in samples),
        "pg_peak_process_rss_sum_bytes": max(
            r["pg_process_rss_sum_bytes"] for r in samples
        ),
        "sampling_target_ms": 100,
        "sampling_max_gap_ms": max(
            (b["seconds"] - a["seconds"]) * 1000 for a, b in zip(samples, samples[1:])
        ),
        "progress": progress,
        "stats_before": before,
        "stats_after": after,
        "model_calls": [dict(c, at_seconds=c["at"] - start) for c in Upstream.calls],
        "paid_model_calls": 0,
    }
    (h.out / (label + "-resources.jsonl")).write_text(
        "".join(encoded(r) + "\n" for r in samples)
    )
    h.stop()
    result["candidate"] = h.sql(
        db, "SELECT id FROM knowledge.candidates ORDER BY created_at DESC LIMIT 1"
    )
    result["storage"] = json.loads(
        h.sql(
            db,
            "SELECT jsonb_build_object('base_bytes',octet_length(base::text),'view_bytes',octet_length(view::text),'input_bytes',octet_length(execution_input::text),'chunks',(SELECT count(*) FROM knowledge.frozen_input_chunks),'chunk_text_bytes',(SELECT coalesce(sum(octet_length(content)),0) FROM knowledge.frozen_input_chunks),'legacy_archive_bytes',(SELECT coalesce(sum(octet_length(archive)),0) FROM knowledge.field_value_archives)) FROM knowledge.candidates ORDER BY created_at DESC LIMIT 1",
        )
    )
    (h.out / (label + "-result.json")).write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        label,
        round(result["api_peak_rss_bytes"] / 2**20, 2),
        "MiB",
        round(first - start, 2),
        "s",
        flush=True,
    )
    return result


def evidence(h, db, candidate):
    frozen = json.loads(
        h.sql(
            db,
            f"SELECT execution_input FROM knowledge.candidates WHERE id='{candidate}'",
        ),
        parse_float=decimal.Decimal,
    )
    if frozen["version"] == 2:
        p = frozen["prepared"]
        facts = {e["summary"]["id"]: e for e in p["facts"]}
        archive = json.loads(
            h.sql(
                db,
                f"SELECT archive FROM knowledge.field_value_archives WHERE candidate_id='{candidate}'",
            ),
            parse_float=decimal.Decimal,
        )
        result = {
            "summaries": digest(frozen["summaries"]),
            "links": digest(p["links"]),
            "relations": len(p["links"]),
            "relation_environments": sorted(
                set(str(l["context"].get("environment")) for l in p["links"])
            ),
            "endpoints": {},
        }
        for eid, f in facts.items():
            values = {
                "version": 1,
                "fields": {
                    k: v
                    for k, v in archive["fields"].items()
                    if k.startswith(eid + "/")
                },
                "documents": {
                    k: v
                    for k, v in archive["documents"].items()
                    if v["source"].get("endpoint") == eid
                },
            }
            result["endpoints"][eid] = {
                "facts": digest(f),
                "inputs": digest(p["inputs"][eid]),
                "authentication": digest(p["authentication"].get(eid, [])),
                "values": digest(values),
                "fields": len(p["inputs"][eid]),
            }
    else:
        m = frozen["manifest"]
        chunks = {}

        # Driver memory is outside the API observation window. Read one immutable text block at a time.
        def read(ref):
            if ref["sha256"] not in chunks:
                raw = h.sql(
                    db,
                    f"SELECT content FROM knowledge.frozen_input_chunks WHERE project_id='{m['project']}' AND candidate_id='{candidate}' AND snapshot_id='{m['snapshot']}' AND sha256='{ref['sha256']}'",
                )
                assert (
                    hashlib.sha256(raw.encode()).hexdigest() == ref["sha256"]
                    and len(raw.encode()) == ref["bytes"]
                )
                return json.loads(raw, parse_float=decimal.Decimal)
            return chunks[ref["sha256"]]

        links = read(m["links"])
        result = {
            "summaries": digest(frozen["summaries"]),
            "links": digest(links),
            "relations": len(links),
            "relation_environments": sorted(
                set(str(l["context"].get("environment")) for l in links)
            ),
            "endpoints": {},
        }
        for eid, refs in m["endpoints"].items():
            f = read(refs["facts"])
            inputs = (
                [i for b in refs["inputs"] for i in read(b["content"])]
                if isinstance(refs["inputs"], list)
                else read(refs["inputs"])
            )
            values = read(refs["values"])
            values["documents"] = {k: read(v) for k, v in refs["documents"].items()}
            result["endpoints"][eid] = {
                "facts": digest(f["facts"]),
                "inputs": digest(inputs),
                "authentication": digest(f["authentication"]),
                "values": digest(values),
                "fields": len(inputs),
            }
    return result


def stats(h, db):
    return json.loads(
        h.sql(
            db,
            "SELECT jsonb_build_object('db',to_jsonb(d),'wal',(SELECT to_jsonb(w) FROM pg_stat_wal w),'database_bytes',pg_database_size(current_database())) FROM pg_stat_database d WHERE datname=current_database()",
        )
    )
