#!/usr/bin/env python3
"""Merge a new measurement of the two histogram kernels into a report.

    bench/merge_hist.py NAME NEW NOTE

replaces the histogram runs of results/NAME.json by those of
results/NEW.json, sets the note printed at the top of the report (for
example "measured again on <date> after DECISIONS D78"), and removes NEW.
The report is then written again with `--report-only` (bench/run.py or
bench/parallel.py, whichever produced NAME)."""
import json, os, sys

here = os.path.dirname(os.path.abspath(__file__))
results = os.path.join(here, "..", "results")
name, new, note = sys.argv[1:4]
path = os.path.join(results, name + ".json")
res = json.load(open(path))
fresh = json.load(open(os.path.join(results, new + ".json")))
for size, kernels in fresh["runs"].items():
    for k in ("histogram", "histogram_skewed"):
        if k in kernels:
            res["runs"][size][k] = kernels[k]
res["note_hist"] = note
json.dump(res, open(path, "w"), indent=1)
for ext in (".json", ".md"):
    p = os.path.join(results, new + ext)
    if os.path.exists(p):
        os.remove(p)
print("merged", new, "into", name)
