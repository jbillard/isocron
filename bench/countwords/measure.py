#!/usr/bin/env python3
"""countwords (benhoyt/countwords, Rust by Andrew Gallant) and its port to
Isocron (bench/countwords/isocron): the times and the memory of the three
programs on the same input (the King James Bible ten times, 43 MB), their
outputs checked the same (ordered by count, then by word, as the original
`normalize.py` orders them).

Under the protocol of the machine (bench/machine.sh):

  bench/countwords/measure.py input       (the input, fetched and checked)
  bench/machine.sh run --script bench/countwords/measure.py build
  bench/machine.sh run --script bench/countwords/measure.py run
  bench/countwords/measure.py memory      (the peak resident memory)
  bench/countwords/measure.py report      (results/countwords.md)

Each step writes bench/countwords/results/<step>.json. `run`: the Rust
programs (one thread each, as written) and Isocron on one thread and on
the CPUs of the benchmark, interleaved, 11 times each, the median, the
minimum, the quartiles; each output compared with the simple Rust
program's. `memory`: `VmHWM` of each program alone, read every
millisecond, apart from the timed runs.
"""

import argparse
import datetime
import hashlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
import run  # noqa: E402  (machine(), quartiles())

OUT = os.path.join(HERE, "results")
WORK = os.path.join(ROOT, "target", "countwords", "measure")
ISCC = os.path.join(ROOT, "target", "release", "iscc")
CARGO = os.path.expanduser("~/.cargo/bin/cargo")
# the input of the original benchmark, at the commit the Rust programs are
# taken from
COMMIT = "d8c5cf3af2f58f4ca8b4f6d59cfd3329f815c453"
INPUT_URL = "https://raw.githubusercontent.com/benhoyt/countwords/%s/kjvbible.txt" % COMMIT
INPUT_SHA256 = "de6d4f0b2ab6c394412068a7fae50a08490dd8e561f7620074d4984709c322d1"
INPUT = os.path.join(WORK, "kjvbible_x10.txt")
REPS = 11


def log(*a):
    print(*a, file=sys.stderr, flush=True)


def save(name, data):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name + ".json")
    with open(path, "w") as f:
        json.dump(data, f, indent=1)
    log("written", path)


def load(name):
    path = os.path.join(OUT, name + ".json")
    return json.load(open(path)) if os.path.exists(path) else None


def summary(xs):
    q1, med, q3 = run.quartiles(xs) if len(xs) >= 2 else (xs[0], xs[0], xs[0])
    return {"all": xs, "median": med, "min": min(xs), "q1": q1, "q3": q3}


def normalized(data):
    """The lines `word count` ordered by count, most first, then by word."""
    rows = []
    for line in data.decode().splitlines():
        word, count = line.split()
        rows.append((-int(count), word))
    rows.sort()
    return "\n".join("%s %d" % (w, -c) for c, w in rows)


# ----- the input -------------------------------------------------------------------


def input_step(args):
    os.makedirs(WORK, exist_ok=True)
    one = os.path.join(WORK, "kjvbible.txt")
    if not os.path.exists(one):
        with urllib.request.urlopen(INPUT_URL) as r, open(one, "wb") as f:
            shutil.copyfileobj(r, f)
    data = open(one, "rb").read()
    if hashlib.sha256(data).hexdigest() != INPUT_SHA256:
        raise SystemExit("%s: not the expected text (sha256)" % one)
    with open(INPUT, "wb") as f:
        for _ in range(10):
            f.write(data)
    log("written", INPUT, len(data) * 10, "bytes")


# ----- builds ----------------------------------------------------------------------


def programs():
    return [
        ("rust-simple", os.path.join(WORK, "simple", "release", "countwords"), 1),
        ("rust-optimized", os.path.join(WORK, "optimized", "release", "countwords"), 1),
        ("isc-1", os.path.join(WORK, "isc", "countwords"), 1),
        ("isc-n", os.path.join(WORK, "isc", "countwords"), 0),
    ]


def timed(cmd, cwd=None, env=None):
    t0 = time.perf_counter()
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    _, status, ru = os.wait4(p.pid, 0)
    wall = time.perf_counter() - t0
    err = p.stderr.read().decode()
    p.stderr.close()
    if status != 0:
        raise RuntimeError("%s failed: %s" % (" ".join(cmd), err[-2000:]))
    return wall, ru.ru_utime + ru.ru_stime


def build(args):
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "machine": run.machine(), "builds": {}}
    rows = {}
    for name in ("simple", "optimized"):
        target = os.path.join(WORK, name)

        def cargo(target=target, name=name):
            if os.path.isdir(target):
                shutil.rmtree(target)
            env = dict(os.environ, CARGO_TARGET_DIR=target)
            return timed([CARGO, "build", "--release", "--quiet"], cwd=os.path.join(HERE, "rust", name), env=env)

        rows["rust-" + name] = cargo
    os.makedirs(os.path.join(WORK, "isc"), exist_ok=True)
    rows["isc"] = lambda: timed([ISCC, "build", os.path.join(HERE, "isocron", "countwords.isc"), "-o", os.path.join(WORK, "isc", "countwords")])
    for name, f in rows.items():
        walls, cpus = [], []
        for k in range(3):
            wall, cpu = f()
            log("%s %d: %.2f s (processor %.2f s)" % (name, k, wall, cpu))
            walls.append(wall)
            cpus.append(cpu)
        res["builds"][name] = {"wall": summary(walls), "cpu": summary(cpus)}
    res["sizes"] = {name: os.path.getsize(exe) for name, exe, _ in programs()[:3]}
    res["versions"] = {"rustc": run.sh("%s --version" % os.path.join(os.path.dirname(CARGO), "rustc")), "iscc": run.sh("git -C %s log --format=%%h -1" % ROOT)}
    save("build", res)


# ----- runs ------------------------------------------------------------------------


def cpus_for(threads):
    cpus = os.environ.get("ISC_BENCH_CPUS", "")
    if threads == 1:
        return os.environ.get("ISC_BENCH_CPU", cpus.split(",")[0] if cpus else "0")
    return cpus


def run_step(args):
    ref = None
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "machine": run.machine(), "cpus": cpus_for(0), "programs": {}}
    rows = {name: [] for name, _, _ in programs()}
    same = {name: True for name, _, _ in programs()}
    out = os.path.join(WORK, "out.txt")
    for k in range(REPS):
        for name, exe, threads in programs():
            cpus = cpus_for(threads)
            env = dict(os.environ, ISC_THREADS=str(threads or len(cpus.split(","))))
            with open(INPUT, "rb") as f, open(out, "wb") as o:
                t0 = time.perf_counter()
                subprocess.run(["taskset", "-c", cpus, exe], stdin=f, stdout=o, env=env, check=True)
                rows[name].append(time.perf_counter() - t0)
            text = normalized(open(out, "rb").read())
            if ref is None:
                ref = text
            same[name] = same[name] and text == ref
            log("%s %d: %.3f s%s" % (name, k, rows[name][-1], "" if same[name] else ", OTHER OUTPUT"))
    for name, _, threads in programs():
        res["programs"][name] = {"threads": threads or len(res["cpus"].split(",")), "wall": summary(rows[name]), "same_output": same[name]}
    save("run", res)


# ----- memory ----------------------------------------------------------------------


def peak_memory(cmd, env):
    """The peak resident memory of `cmd` alone, in KiB (`VmHWM`, read every
    millisecond until it exits)."""
    with open(INPUT, "rb") as f:
        p = subprocess.Popen(cmd, stdin=f, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        hwm = 0
        while p.poll() is None:
            try:
                for line in open("/proc/%d/status" % p.pid):
                    if line.startswith("VmHWM:"):
                        hwm = max(hwm, int(line.split()[1]))
            except (FileNotFoundError, ProcessLookupError, ValueError):
                pass
            time.sleep(0.001)
    if p.returncode != 0:
        raise RuntimeError("%s failed (%d)" % (" ".join(cmd), p.returncode))
    return hwm


def memory(args):
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "programs": {}}
    for name, exe, threads in programs():
        env = dict(os.environ, ISC_THREADS=str(threads or 8))
        kib = max(peak_memory([exe], env) for _ in range(3))
        log(name, kib, "KiB")
        res["programs"][name] = {"threads": threads or 8, "kib": kib}
    save("memory", res)


# ----- report ----------------------------------------------------------------------


def report(args):
    names = {"rust-simple": "Rust, simple", "rust-optimized": "Rust, optimized", "isc-1": "Isocron, one thread", "isc-n": "Isocron, %s threads",
             "isc": "Isocron (`iscc build`)", "rust-simple-build": "Rust, simple", "rust-optimized-build": "Rust, optimized"}
    lines = ["# countwords: Rust and Isocron", ""]
    b = load("build")
    if b:
        m = b["machine"]
        lines += ["%s, %s, %s; %s; Isocron %s. The input: the King James Bible ten times (43 MB)." % (m["cpu"], m["os"], m["kernel"], b["versions"]["rustc"], b["versions"]["iscc"]), ""]
        lines += ["## Builds", "", "| Build | Wall (median) | Processor (median) |", "| --- | ---: | ---: |"]
        for k, r in b["builds"].items():
            lines.append("| %s | %.2f s | %.2f s |" % (names.get(k, k) + (", clean" if k.startswith("rust") else ""), r["wall"]["median"], r["cpu"]["median"]))
        lines += ["", "| Program | Size |", "| --- | ---: |"]
        for k, n in b["sizes"].items():
            lines.append("| %s | %d KiB |" % ({"isc-1": "Isocron"}.get(k, names.get(k, k)), n // 1024))
        lines.append("")
    r = load("run")
    if r:
        lines += ["## Runs", "", "| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |", "| --- | ---: | ---: | ---: | ---: | --- |"]
        for name, x in r["programs"].items():
            label = names.get(name, name)
            if "%s" in label:
                label = label % x["threads"]
            w = x["wall"]
            lines.append("| %s | %d | %.3f s | %.3f s | %.3f to %.3f s | %s |" % (label, x["threads"], w["median"], w["min"], w["q1"], w["q3"], "yes" if x["same_output"] else "NO"))
        lines.append("")
    mem = load("memory")
    if mem:
        lines += ["## Peak resident memory (`VmHWM`)", "", "| Program | Threads | Peak |", "| --- | ---: | ---: |"]
        for name, x in mem["programs"].items():
            label = names.get(name, name)
            if "%s" in label:
                label = label % x["threads"]
            lines.append("| %s | %d | %.1f MiB |" % (label, x["threads"], x["kib"] / 1024.0))
        lines.append("")
    path = os.path.join(OUT, "countwords.md")
    open(path, "w").write("\n".join(lines))
    log("written", path)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("step", choices=["input", "build", "run", "memory", "report"])
    args = ap.parse_args()
    {"input": input_step, "build": build, "run": run_step, "memory": memory, "report": report}[args.step](args)


if __name__ == "__main__":
    main()
