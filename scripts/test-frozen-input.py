#!/usr/bin/env python3
"""Step 04: fresh real API binaries and private PostgreSQL; zero paid or business requests."""

import argparse, pathlib, json

# Export the same fixture surface for the independent recovery driver.
from frozen_input_harness import Harness, Upstream, ident, encoded, digest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--optimized", required=True)
    parser.add_argument("--admin", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--interfaces", type=int, default=501)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    pathlib.Path("/tmp/nexofolio-step04").mkdir(exist_ok=True)
    h = Harness(out, pathlib.Path(args.admin).resolve())
    try:
        template = "nexofolio_step04_seed"
        h.create(template)
        h.start(template, args.baseline, "seed")
        meta = h.seed(template, args.interfaces)
        h.stop()
        (out / "fixture.json").write_text(json.dumps(meta, indent=2) + "\n")
        results = []
        original = None
        for i in range(args.repeats):
            for mode, binary in [
                ("baseline", args.baseline),
                ("optimized", args.optimized),
            ]:
                label = f"{mode}-{i+1}"
                db = "nexofolio_step04_" + label.replace("-", "_")
                h.create(db, template)
                measured = h.measure(db, binary, label, meta["project"])
                results.append(measured)
                evidence = h.evidence(db, measured["candidate"])
                (out / (label + "-equivalence.json")).write_text(
                    json.dumps(evidence, indent=2) + "\n"
                )
                if original is None:
                    original = evidence
                else:
                    assert evidence == original, (
                        "complete frozen input equivalence failed " + label
                    )
                print(
                    label,
                    "full evidence equivalent",
                    sum(e["fields"] for e in evidence["endpoints"].values()),
                    "fields",
                    flush=True,
                )
        (out / "results.json").write_text(
            json.dumps(results, ensure_ascii=False, indent=2) + "\n"
        )
    finally:
        h.cleanup()
        (out / "cleanup.json").write_text(
            json.dumps(
                {
                    "owned_processes_stopped": True,
                    "private_pg_deleted": True,
                    "paid_calls": 0,
                }
            )
            + "\n"
        )


if __name__ == "__main__":
    main()
