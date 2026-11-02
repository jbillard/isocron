#!/usr/bin/env python3
"""The summary of a campaign: results/<name>-summary.md from
results/<name>-single.json (bench/run.py), results/<name>.json
(bench/parallel.py) and results/<name>-compile.json (bench/compile.py).

    bench/summary.py stage2

Every number comes from the JSON files; the verdicts follow SPEC section
14 (bench/run.py, `verdict`)."""
import json, os, statistics, sys

here = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, here)
import run  # noqa: E402

RESULTS = os.path.join(here, "..", "results")


def fmt(ns):
    return run.fmt_ns(ns)


def pct(a, b):
    return "%+.1f %%" % (100.0 * (a / b - 1.0))


def main():
    name = sys.argv[1]
    single = json.load(open(os.path.join(RESULTS, name + "-single.json")))
    par = json.load(open(os.path.join(RESULTS, name + ".json")))
    comp = json.load(open(os.path.join(RESULTS, name + "-compile.json")))
    L = []
    w = L.append
    w("# Isocron — summary of the campaign `%s`" % name)
    w("")
    w("One core: %s; threads: %s; compilation: %s (machine %s). Detailed reports:" % (
        single["date"], par["date"], comp["date"], par.get("host", "?")))
    w("[%s-single.md](%s-single.md), [%s.md](%s.md), [%s-compile.md](%s-compile.md). Verdict rule of SPEC" % ((name,) * 6))
    w("section 14: *beats* when Isocron's median is lower by more than the dispersion (the larger IQR of the")
    w("two), *equals* within the dispersion, *slower* otherwise.")
    w("")
    for note in par.get("notes", []) + single.get("notes", []):
        w("Note: %s." % note)
        w("")
    # ---- one core, strict
    w("## One core: Isocron against the best strict variant of the other languages")
    w("")
    w("| Kernel | Size | Isocron | Best other | Difference | Verdict |")
    w("| --- | --- | --- | --- | --- | --- |")
    counts = {"beats": 0, "equals": 0, "slower": 0}
    lowest = 0
    total = 0
    for size, kernels in single["runs"].items():
        for kernel, rows in kernels.items():
            isc = run.best(rows, lambda r: r["kind"] == "isocron")
            other = run.best(rows, lambda r: r["kind"] == "strict")
            v = run.verdict(isc, other)
            counts[v] += 1
            total += 1
            lowest += isc["median"] <= other["median"]
            w("| %s | %s | %s | %s (%s, %s) | %s | %s |" % (kernel, size, fmt(isc["median"]), fmt(other["median"]),
                                                          other["label"], other["variant"], pct(isc["median"], other["median"]),
                                                          "**%s**" % v if v == "beats" else v))
    w("")
    w("Isocron has the lowest median in %d of the %d cases (%d *beats*, %d *equals*, %d *slower*)." % (
        lowest, total, counts["beats"], counts["equals"], counts["slower"]))
    w("")
    # ---- one core, reassociated
    w("## One core: `allow_reassoc` against the reassociated variants")
    w("")
    w("| Kernel | Size | Isocron allow_reassoc | Best reassociated | Difference | Verdict |")
    w("| --- | --- | --- | --- | --- | --- |")
    rl = rt = 0
    for size, kernels in single["runs"].items():
        for kernel, rows in kernels.items():
            isc = run.best(rows, lambda r: r["kind"] == "isocron-reassoc")
            other = run.best(rows, lambda r: r["kind"] == "reassoc")
            if not isc or not other:
                continue
            v = run.verdict(isc, other)
            rt += 1
            rl += isc["median"] <= other["median"]
            w("| %s | %s | %s | %s (%s) | %s | %s |" % (kernel, size, fmt(isc["median"]), fmt(other["median"]),
                                                     other["label"], pct(isc["median"], other["median"]),
                                                     "**%s**" % v if v == "beats" else v))
    w("")
    w("Isocron's `allow_reassoc` programs have the lowest median in %d of the %d cases." % (rl, rt))
    w("")
    # ---- comparison B
    b = [(size, r) for size, ks in single["runs"].items() for r in ks.get("moving_average_b", []) if "exact_rel" in r]
    if b:
        w("## Comparison B: the moving average, distance to the exact sums of the windows")
        w("")
        w("| Size | Implementation | Variant | Largest absolute difference | Largest relative difference |")
        w("| --- | --- | --- | --- | --- |")
        for size, r in b:
            w("| %s | %s | %s | %.2e | %.2e |" % (size, r["label"], r["variant"], r["exact_abs"], r["exact_rel"]))
        w("")
    # ---- threads
    w("## Threads: Isocron against C (OpenMP), Rust (rayon), Go and Zig")
    w("")
    w("Best parallel variant of the other languages for the same number of threads T; the difference is")
    w("Isocron's median against it.")
    w("")
    ths = par["threads"]
    w("| Kernel | Size | " + " | ".join("T=%d" % t for t in ths) + " |")
    w("|" + " --- |" * (len(ths) + 2))
    pl = pt = 0
    lost = []
    for size, kernels in par["runs"].items():
        for kernel, rows in kernels.items():
            cells = []
            for t in ths:
                rs = [r for r in rows if r["threads"] == t]
                isc = min((r for r in rs if r["impl"] == "isc"), key=lambda r: r["median"])
                other = min((r for r in rs if r["impl"] != "isc"), key=lambda r: r["median"])
                v = run.verdict(isc, other)
                pt += 1
                if isc["median"] <= other["median"]:
                    pl += 1
                else:
                    lost.append((kernel, size, t, isc, other))
                cells.append("%s (%s)" % (v, pct(isc["median"], other["median"])))
            w("| %s | %s | %s |" % (kernel, size, " | ".join(cells)))
    w("")
    w("Isocron has the lowest median in %d of the %d cases." % (pl, pt))
    for kernel, size, t, isc, other in lost:
        w("Not in: %s, %s, T=%d: Isocron %s, %s %s (dispersion %s)." % (
            kernel, size, t, fmt(isc["median"]), other["bin"], fmt(other["median"]), fmt(max(isc["iqr"], other["iqr"]))))
    w("")
    # ---- compilation
    w("## Compilation times")
    w("")
    w("Median over the programs of each implementation; *warm*: the source changed and built again; *cold*:")
    w("every cache empty (GCC and Isocron have no cache).")
    w("")
    w("| Implementation | Programs | Warm wall | Cold wall |")
    w("| --- | --- | --- | --- |")
    by = {}
    for p in comp["programs"]:
        by.setdefault(p["impl"], []).append(p)
    labels = {"isc": "Isocron", "c-gcc": "C (GCC)", "c-clang": "C (Clang, zig cc)", "rust": "Rust", "go": "Go", "zig": "Zig"}
    for impl in ("isc", "c-gcc", "c-clang", "rust", "go", "zig"):
        ps = by.get(impl, [])
        if not ps:
            continue
        warm = statistics.median(statistics.median(p["warm_wall"]) for p in ps)
        cold = statistics.median(statistics.median(p["cold_wall"]) for p in ps)
        w("| %s | %d | %.2f s | %s |" % (labels[impl], len(ps), warm, "same (no cache)" if ps[0].get("no_cache") else "%.2f s" % cold))
    w("")
    w("Isocron per program, warm: " + ", ".join("`%s` %.3f s" % (p["kernel"], statistics.median(p["warm_wall"])) for p in by.get("isc", [])) + ".")
    w("")
    out = os.path.join(RESULTS, name + "-summary.md")
    open(out, "w").write("\n".join(L) + "\n")
    print("wrote", out)


if __name__ == "__main__":
    main()
