#!/usr/bin/env python3
"""An A/B of two compilers over the programs that watch for regressions
(D727): the Benchmarks Game, walk, countwords, rtiow (its two scenes, on 1
and on 8 threads) and the groups of bench/primitives. Each program is built
by both compilers, then:

- its time: the two builds alternated, RUNS times (rtiow on 1 thread 3), the
  median of each; the outputs of the two builds compared (the image of
  rtiow, the checksums of the primitives);
- its peak resident memory: `VmHWM` of /proc/<pid>/status read every
  millisecond, the largest of 3 runs, the program run without huge pages
  (prctl PR_SET_THP_DISABLE, inherited by its exec): the system gives huge
  pages or not by the state of its memory, and a peak measured with them
  moves by 2 MiB from one run to the next (rtiow: 1.3 or 5.3 MiB).

A time worse by more than 1 % (a form of the primitives: 3 %), a peak worse
by more than 2 % and 64 KiB, an output that differs: listed at the end as
regressions, exit code 1. Each one is either explained (the measure that
shows its cause) or refused before a push.

  bench/machine.sh run --script bench/regress.py OLD_ISCC NEW_ISCC \\
      [--runs 11] [--prim-runs 11] [--only corpus,countwords,rtiow,memory,primitives]

Under the protocol of the machine (`bench/machine.sh apply single` first).
"""

import argparse
import ctypes
import hashlib
import os
import socket
import statistics
import subprocess
import sys
import time
import types

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = os.path.join(ROOT, "target", "regress")
INPUTS = os.path.join(ROOT, "target", "bench-inputs")
KJV = os.path.join(ROOT, "target", "countwords", "measure", "kjvbible_x10.txt")
BG = os.path.join(ROOT, "bench", "benchmarks_game", "isocron")

# (name, source, arguments, standard input, threads)
CORPUS = [
    ("binarytrees", BG + "/binarytrees.isc", ["21"], None, 4),
    ("fannkuchredux", BG + "/fannkuchredux.isc", ["12"], None, 4),
    ("fasta", BG + "/fasta.isc", ["25000000"], None, 4),
    ("knucleotide", BG + "/knucleotide.isc", ["0"], INPUTS + "/fasta25m.txt", 4),
    ("mandelbrot", BG + "/mandelbrot.isc", ["16000"], None, 4),
    ("nbody", BG + "/nbody.isc", ["50000000"], None, 4),
    ("pidigits", BG + "/pidigits.isc", ["10000"], None, 4),
    ("regexredux", BG + "/regexredux.isc", ["0"], INPUTS + "/fasta5m.txt", 4),
    ("revcomp", BG + "/revcomp.isc", ["0"], INPUTS + "/fasta25m.txt", 4),
    ("spectralnorm", BG + "/spectralnorm.isc", ["5500"], None, 4),
    ("walk", ROOT + "/bench/records/walk.isc", ["1000000"], None, 1),
    ("countwords", ROOT + "/bench/countwords/isocron/countwords.isc", [], KJV, 1),
]
RTIOW = ROOT + "/bench/rtiow/isocron/main.isc"
# (scene, samples, width, height, threads)
RENDERS = [("sphere", 100, 384, 216, 1), ("final", 20, 384, 216, 1), ("sphere", 100, 384, 216, 8), ("final", 20, 384, 216, 8)]

regressions = []


def log(*a):
    print(*a, flush=True)


def build(iscc, src, out):
    os.makedirs(os.path.dirname(out), exist_ok=True)
    subprocess.run([iscc, "build", os.path.basename(src), "-o", out], cwd=os.path.dirname(src), check=True, stdout=subprocess.DEVNULL)


def cpus(threads):
    all_ = os.environ["ISC_BENCH_CPUS"].split(",")
    return os.environ["ISC_BENCH_CPU"] if threads == 1 else ",".join(all_[:threads])


def no_thp():
    ctypes.CDLL(None).prctl(41, 1, 0, 0, 0)


def timed(cmd, stdin, threads, out):
    env = dict(os.environ, ISC_THREADS=str(threads))
    with open(stdin or "/dev/null", "rb") as f, open(out, "wb") as o:
        t0 = time.perf_counter()
        subprocess.run(["taskset", "-c", cpus(threads)] + cmd, stdin=f, stdout=o, env=env, check=True)
        return (time.perf_counter() - t0) * 1000


def peak(cmd, stdin, threads):
    real = os.path.realpath(cmd[0])
    env = dict(os.environ, ISC_THREADS=str(threads))
    with open(stdin or "/dev/null", "rb") as f:
        p = subprocess.Popen(["taskset", "-c", cpus(threads)] + cmd, stdin=f, stdout=subprocess.DEVNULL, env=env, preexec_fn=no_thp)
        hwm = 0
        while p.poll() is None:
            try:
                for pid in [p.pid] + [int(c) for c in open("/proc/%d/task/%d/children" % (p.pid, p.pid)).read().split()]:
                    if os.readlink("/proc/%d/exe" % pid) == real:
                        for line in open("/proc/%d/status" % pid):
                            if line.startswith("VmHWM:"):
                                hwm = max(hwm, int(line.split()[1]))
            except (FileNotFoundError, ProcessLookupError, ValueError, OSError):
                pass
            time.sleep(0.001)
    if p.returncode != 0:
        raise RuntimeError("%s failed (%d)" % (" ".join(cmd), p.returncode))
    return hwm


def compare_time(name, old, new, limit, same_bin=False):
    mo, mn = statistics.median(old), statistics.median(new)
    d = 100 * (mn - mo) / mo
    log("%-28s old %10.1f  new %10.1f  %+6.1f %%  [%.1f-%.1f / %.1f-%.1f]%s" % (name, mo, mn, d, min(old), max(old), min(new), max(new), "  (the same binary)" if same_bin else ""))
    if d > limit and not same_bin:
        regressions.append("%s: time %+.1f %% (%.1f to %.1f)" % (name, d, mo, mn))


def compare_peak(name, old, new):
    d = 100.0 * (new - old) / old
    log("%-28s old %8d KiB  new %8d KiB  %+6.1f %%" % (name, old, new, d))
    if new - old > 64 and d > 2:
        regressions.append("%s: peak %+.1f %% (%d to %d KiB)" % (name, d, old, new))


def programs(a, builds):
    """The corpus, walk and countwords: times, then peaks."""
    for name, src, args, stdin, threads in CORPUS:
        if a.only and name != "countwords" and "corpus" not in a.only:
            continue
        if a.only and name == "countwords" and "countwords" not in a.only and "corpus" not in a.only:
            continue
        runs = {"old": [], "new": []}
        outs = {}
        n = a.runs * 2 if name == "countwords" else a.runs
        for r in range(n):
            for v in ("old", "new") if r % 2 == 0 else ("new", "old"):
                out = os.path.join(WORK, "out-%s-%s" % (name, v))
                runs[v].append(timed([builds[v][name]] + args, stdin, threads, out))
                data = open(out, "rb").read()
                # (walk prints its time: its first line compared)
                outs[v] = data.splitlines()[:1] if name == "walk" else data
        if outs["old"] != outs["new"]:
            regressions.append("%s: the outputs differ" % name)
        compare_time(name, runs["old"], runs["new"], 1.0, open(builds["old"][name], "rb").read() == open(builds["new"][name], "rb").read())
    if not a.only or "memory" in a.only:
        for name, src, args, stdin, threads in CORPUS:
            compare_peak(name + " (peak)", max(peak([builds["old"][name]] + args, stdin, threads) for _ in range(3)),
                         max(peak([builds["new"][name]] + args, stdin, threads) for _ in range(3)))


def rtiow(a, builds):
    """rtiow: its two scenes on 1 and 8 threads, times and peaks, the images
    of the two builds compared."""
    for scene, samples, w, h, threads in RENDERS:
        key = "rtiow %s %d thread%s" % (scene, threads, "s" if threads > 1 else "")
        runs = {"old": [], "new": []}
        digests = {}
        n = 3 if threads == 1 else 5
        for r in range(n):
            for v in ("old", "new") if r % 2 == 0 else ("new", "old"):
                img = os.path.join(WORK, "rtiow-%s.ppm" % v)
                cmd = [builds[v]["rtiow"], img, str(samples), str(w), str(h), "42", scene]
                runs[v].append(timed(cmd, None, threads, os.devnull))
                digests[v] = hashlib.sha256(open(img, "rb").read()).hexdigest()
        if digests["old"] != digests["new"]:
            regressions.append("%s: the images differ" % key)
        compare_time(key, runs["old"], runs["new"], 1.0)
        if not a.only or "memory" in a.only:
            pk = {}
            for v in ("old", "new"):
                img = os.path.join(WORK, "rtiow-mem-%s.ppm" % v)
                pk[v] = max(peak([builds[v]["rtiow"], img, str(samples), str(w), str(h), "42", scene], None, threads) for _ in range(3))
            compare_peak(key + " (peak)", pk["old"], pk["new"])


def primitives(a, compilers):
    """The groups of bench/primitives, each started as bench/primitives/run.py
    starts it (its server, its input, its client); the two builds
    alternated, the median of each form; a form 3 % slower, or whose
    checksums differ: a regression."""
    here = os.path.join(ROOT, "bench", "primitives")
    # (its functions alone: the file runs its `main` when it is executed)
    path = os.path.join(here, "run.py")
    src = open(path).read().split("\ndef main():")[0]
    defs = {"__file__": path, "__name__": "prim_run_defs"}
    exec(compile(src, path, "exec"), defs)
    R = types.SimpleNamespace(**defs)
    cpu = R.bench_cpu()
    groups = sorted(f[:-4] for f in os.listdir(here) if f.endswith(".isc") and os.path.exists(os.path.join(here, f[:-4] + ".c")))
    for g in groups:
        text = open(os.path.join(here, g + ".isc")).read()
        bins = {}
        for v, iscc in compilers.items():
            bins[v] = os.path.join(WORK, v, "prim-" + g)
            build(iscc, os.path.join(here, g + ".isc"), bins[v])
        libs = ["-l:libssl.so.3", "-l:libcrypto.so.3"] if "// bench-server: tls" in text or "// bench-client: tls" in text else []
        server, extra = None, []
        if "// bench-server: tls" in text:
            server, extra = R.tls_server()
        elif "// bench-server: http" in text:
            server, extra = R.http_server()
        elif "// bench-server: dns" in text:
            server, extra = R.dns_server()
        served = None
        if "// bench-client: http" in text:
            client = os.path.join(R.build, "http_client")
            subprocess.run(["gcc", "-O2", os.path.join(here, "http_client.c"), "-o", client], check=True)
            served = (None, client)
        elif "// bench-client: tls" in text:
            tls, _ = R.tls_server()
            tls.kill()
            tls.wait()
            client = os.path.join(R.build, "https_client")
            subprocess.run(["gcc", "-O2", os.path.join(here, "https_client.c"), "-o", client] + libs, check=True)
            served = (os.path.join(R.build, "tls"), client)
        res = {"old": {}, "new": {}}
        rounds = str(a.rounds)
        try:
            for r in range(a.prim_runs):
                for v in ("old", "new") if r % 2 == 0 else ("new", "old"):
                    b = bins[v]
                    if served:
                        dd, client = served
                        s = socket.socket()
                        s.bind(("127.0.0.1", 0))
                        port = s.getsockname()[1]
                        s.close()
                        if dd is None:
                            lines = R.run_served([b, rounds, str(port)], cpu, port, [client, rounds, str(port)])
                        else:
                            args = [rounds, str(port), os.path.join(dd, "server.pem"), os.path.join(dd, "server.key")]
                            lines = R.run_served([b] + args, cpu, port, [client, rounds, str(port), os.path.join(dd, "ca.pem")])
                    elif "// bench-io" in text:
                        lines = R.run_io([b, rounds], cpu)
                    else:
                        env = ["SSL_CERT_FILE=" + extra[1]] if "// bench-server: tls" in text else []
                        lines = R.run([b, rounds] + extra, cpu, env)
                    for name, (ns, chk) in lines.items():
                        res[v].setdefault(name, []).append((ns, chk))
        finally:
            if server:
                server.kill()
                server.wait()
        # (two binaries the same byte for byte: what moves is the machine)
        same_bin = open(bins["old"], "rb").read() == open(bins["new"], "rb").read()
        moved = 0
        for name in res["old"]:
            o = [x for x, _ in res["old"][name]]
            n = [x for x, _ in res["new"].get(name, [])]
            if not n:
                regressions.append("%s %s: no longer measured" % (g, name))
                continue
            if {c for _, c in res["old"][name]} != {c for _, c in res["new"][name]}:
                regressions.append("%s %s: the checksums differ" % (g, name))
            mo, mn = statistics.median(o), statistics.median(n)
            d = 100 * (mn - mo) / mo
            if abs(d) >= 3:
                moved += 1
                log("%-8s %-24s %10.3f -> %10.3f  %+6.1f %%  [%.3f-%.3f / %.3f-%.3f]" % (g, name, mo, mn, d, min(o), max(o), min(n), max(n)))
            if d >= 3 and not same_bin:
                regressions.append("%s %s: %+.1f %% (%.3f to %.3f ns)" % (g, name, d, mo, mn))
        log("%-8s %d forms, %d moved by 3 %% or more%s" % (g, len(res["old"]), moved, " (the same binary)" if same_bin else ""))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--runs", type=int, default=11)
    ap.add_argument("--prim-runs", type=int, default=11)
    ap.add_argument("--rounds", type=int, default=200)
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    a.only = [x for x in a.only.split(",") if x]
    compilers = {"old": os.path.realpath(a.old), "new": os.path.realpath(a.new)}
    builds = {"old": {}, "new": {}}
    for v, iscc in compilers.items():
        for name, src, _, _, _ in CORPUS:
            builds[v][name] = os.path.join(WORK, v, name)
            build(iscc, src, builds[v][name])
        builds[v]["rtiow"] = os.path.join(WORK, v, "rtiow")
        build(iscc, RTIOW, builds[v]["rtiow"])
    log("old: %s\nnew: %s" % (compilers["old"], compilers["new"]))
    if not a.only or any(x in a.only for x in ("corpus", "countwords", "memory")):
        programs(a, builds)
    if not a.only or "rtiow" in a.only:
        rtiow(a, builds)
    if not a.only or "primitives" in a.only:
        primitives(a, compilers)
    if regressions:
        log("\nREGRESSIONS (%d):" % len(regressions))
        for r in regressions:
            log("  " + r)
        sys.exit(1)
    log("\nNO REGRESSION")


if __name__ == "__main__":
    main()
