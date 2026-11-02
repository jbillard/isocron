#!/usr/bin/env python3
"""Merge a new measurement of some kernels into a report.

    bench/merge_kernels.py NAME NEW KERNELS NOTE

replaces the runs of the kernels KERNELS (comma-separated) in
results/NAME.json by those of results/NEW.json, adds NOTE to the notes
printed at the top of the report (for example "matvec measured again on
<date> after DECISIONS D121"), and removes NEW. The report is then written
again with `--report-only` (bench/run.py or bench/parallel.py, whichever
produced NAME). The general form of bench/merge_hist.py."""
import json, os, sys

here = os.path.dirname(os.path.abspath(__file__))
results = os.path.join(here, "..", "results")
name, new, kernels, note = sys.argv[1:5]
path = os.path.join(results, name + ".json")
res = json.load(open(path))
fresh = json.load(open(os.path.join(results, new + ".json")))
for size, ks in fresh["runs"].items():
    for k in kernels.split(","):
        if k in ks:
            res["runs"][size][k] = ks[k]
res.setdefault("notes", []).append(note)
json.dump(res, open(path, "w"), indent=1)
for ext in (".json", ".md"):
    p = os.path.join(results, new + ext)
    if os.path.exists(p):
        os.remove(p)
print("merged", new, "into", name)
