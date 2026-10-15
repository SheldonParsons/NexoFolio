#!/usr/bin/env python3
"""Summarize saved ledgers without rerunning the benchmark. Nearest-rank percentiles."""
import argparse
import collections
import json
import math
import pathlib
import statistics


def distribution(values):
    values = sorted(values)
    return {"n": len(values), "p50": statistics.median(values), "p95": values[math.ceil(len(values)*0.95)-1], "max": max(values), "min": min(values), "mean": statistics.mean(values)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=pathlib.Path)
    args = parser.parse_args()
    root = args.directory
    groups = collections.defaultdict(list)
    for file in sorted(root.glob("*-result.json")):
        result = json.loads(file.read_text())
        label = result["label"]
        ledger = json.loads((root / (label + "-ledger.json")).read_text())
        probes = json.loads((root / (label + "-probes.json")).read_text())
        detailed = []
        read, write = {}, {}
        for row in probes:
            if row["kind"] == "read":
                read = row
            if row["kind"] == "write":
                write = row
            if row["kind"] == "append" and row["commands"] == result["command_count"]:
                phases, previous = {}, 0
                for key, mark in row["marks_ms"]:
                    phases[key] = mark - previous
                    previous = mark
                detailed.append({"phases": phases, "read": read, "write": write})
        detailed = detailed[-len(ledger):]
        after = json.loads((root / (label + "-sql-after.json")).read_text())
        sql = {}
        for row in after["statements"]:
            query = " ".join(row["query"].split())
            category = None
            if query.startswith("SELECT id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches") or query.startswith("SELECT state,progress->>"):
                category = "projection_read"
            elif query.startswith("UPDATE candidates SET state=$3,updated_at=$4,view=$5,batches=$6") or query.startswith("UPDATE candidates SET updated_at=$3,view=$4,"):
                category = "projection_write"
            elif query.startswith("SELECT pg_advisory_xact_lock"):
                category = "project_lock"
            if category:
                sql[category] = {key: row.get(key) for key in ["calls", "total_exec_time", "mean_exec_time", "max_exec_time", "shared_blks_hit", "shared_blks_read", "shared_blks_dirtied", "shared_blks_written", "wal_records", "wal_fpi", "wal_bytes", "shared_blk_read_time", "shared_blk_write_time", "temp_blks_written"]}
        run = {"label": label, "first_ms": ledger[0]["apply_ms"], "warmup_ms": [r["apply_ms"] for r in ledger[:10]], "all_apply_ms": distribution([r["apply_ms"] for r in ledger]), "steady_apply_ms": distribution([r["apply_ms"] for r in ledger[10:]]), "steady_apply_checkpoint_ms": distribution([r["apply_checkpoint_ms"] for r in ledger[10:]]), "rss_peak_mib": result["rss_peak_bytes"]/2**20, "rss_before_mib": result["rss_before_bytes"]/2**20, "storage_initial": result["initial"], "storage_final": result["final"], "sql": sql, "steady_values": [r["apply_ms"] for r in ledger[10:]], "steady_ack_values": [r["apply_checkpoint_ms"] for r in ledger[10:]]}
        if detailed:
            run["phase_mean_ms"] = {k: statistics.mean(r["phases"][k] for r in detailed[10:]) for k in detailed[0]["phases"]}
            run["adapter_mean_ms"] = {k: statistics.mean(r[part][k] for r in detailed[10:]) for part, keys in [("read", ["fetch_ms", "decode_ms"]), ("write", ["values_ms", "bind_ms", "execute_ms"])] for k in keys}
        groups[label.rsplit("-r", 1)[0]].append(run)
    summary = {}
    for name, runs in groups.items():
        summary[name] = {"runs": runs, "steady_apply_ms": distribution([x for r in runs for x in r["steady_values"]]), "steady_apply_checkpoint_ms": distribution([x for r in runs for x in r["steady_ack_values"]]), "rss_peak_range_mib": [min(r["rss_peak_mib"] for r in runs), max(r["rss_peak_mib"] for r in runs)]}
        for r in runs:
            del r["steady_values"], r["steady_ack_values"]
    (root / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2)+"\n")
    for name, value in summary.items():
        print(name, value["steady_apply_ms"], value["rss_peak_range_mib"])


if __name__ == "__main__":
    main()
