#!/usr/bin/env python3
"""The checks of a part of the runtime that moves (D617): the same program
built by the compiler of before and by the one of after, compared on every
point a replacement must hold (memory note "replacement checks", D605,
D609, D611, D614, D616). One command for each, the same for an agent and
for its review, each number with the command that gave it.

    replace.py counts  OLD NEW FORMS            instructions an element, outputs
    replace.py cycles  OLD NEW FORMS [NAMES]    cycles an element, pinned core
    replace.py memory  NEW FORMS                most memory at r and 10 r rounds
    replace.py interp  OLD NEW FORMS            ns an element under `iscc run`
    replace.py compile OLD NEW PROGRAM...       instructions of the compiler, sizes
    replace.py same    OLD NEW PROGRAM INPUT... outputs on inputs (and interpreted)

OLD and NEW: two `iscc` binaries (copied first: the suite rebuilds its
own). FORMS: a file of lines `name n r1 r2 pilot.isc arg...`, `ROUNDS` in
the place of the number of rounds, `''` for an empty argument (examples:
`bench/replace/*.forms`); `n` the elements of a round (a pilot
prints `<name> <ns an element> <values...>`). The two binaries of a pilot
are built in the same command, just before they run (D609: a file of
`/tmp` written long before is mapped page by page).

`counts` is deterministic: the instructions of one thread, the difference
of two numbers of rounds over the elements between them; it is the bar of
a replacement, `cycles` only where its margin is thin (under 3 %) or the
form waits on memory. `cycles` needs `sudo -n bench/machine.sh apply
parallel` before and `restore` after, no `Plex Transcoder`, and gives the
median of 9 pairs on the CPU of `bench/build/machine.env`. The caller
holds `target/heavy.lock` while any of them runs: one heavy job at a time
on the machine.
"""
import hashlib
import os
import resource
import statistics
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
USER = os.environ.get("USER", "joe")
WORK = tempfile.mkdtemp(prefix="replace-", dir=os.environ.get("ISC_REPLACE_DIR", os.path.join(ROOT, "target")))


def forms(path):
    out = []
    for line in open(path):
        f = line.split()
        if not f or f[0].startswith("#"):
            continue
        out.append({"name": f[0], "n": int(f[1]), "r1": int(f[2]), "r2": int(f[3]), "pilot": f[4], "args": f[5:]})
    return out


def build(iscc, src, tag, env=None):
    out = os.path.join(WORK, "%s_%s" % (os.path.basename(src).rsplit(".", 1)[0], tag))
    e = dict(os.environ, **(env or {}))
    e.setdefault("ISC_CACHE_DIR", os.path.join(WORK, "cache_" + tag))
    p = subprocess.run([iscc, "build", os.path.join(ROOT, src) if not os.path.isabs(src) else src, "-o", out], capture_output=True, text=True, env=e)
    if p.returncode != 0:
        sys.exit("build %s with %s: %s" % (src, iscc, p.stderr[:400]))
    return out


def args_of(form, rounds):
    # (`''`: an empty argument)
    return [str(rounds) if a == "ROUNDS" else "" if a == "''" else a for a in form["args"]]


def perf(cmd, events="instructions:u", pin=None):
    """The counts of the events of a run, and its standard output."""
    out = os.path.join(WORK, "perf.%d.txt" % os.getpid())
    if pin is None:
        run = ["sudo", "-n", "-u", USER, "env", "ISC_THREADS=1"] + cmd
    else:
        run = ["sudo", "-n", "systemd-run", "--quiet", "--scope", "--slice=isc-bench", "-p", "AllowedCPUs=" + pin,
               "--uid=%d" % os.getuid(), "--gid=%d" % os.getgid(), "-E", "ISC_THREADS=1"] + cmd
    p = subprocess.run(["sudo", "-n", "perf", "stat", "-x,", "-e", events, "-o", out, "--"] + run, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit("%s: exit %d %s" % (" ".join(cmd), p.returncode, p.stderr[:300]))
    v = {}
    for l in open(out):
        f = l.split(",")
        if len(f) > 2 and f[0].replace(".", "").isdigit():
            v[f[2].split(":")[0]] = float(f[0])
    return v, p.stdout


def shown(stdout):
    """The output of a pilot without its column of time (the second)."""
    lines = []
    for l in stdout.splitlines():
        f = l.split()
        lines.append(" ".join(f[:1] + f[2:]))
    return hashlib.md5("\n".join(lines).encode()).hexdigest()[:8]


def per_element(binary, form, events="instructions:u", pin=None):
    a, _ = perf([binary] + args_of(form, form["r1"]), events, pin)
    b, out = perf([binary] + args_of(form, form["r2"]), events, pin)
    k = (form["r2"] - form["r1"]) * form["n"]
    return {e: (b[e] - a[e]) / k for e in a}, out


def pilots(old, new, fs, env=None):
    bins = {}
    for src in sorted(set(f["pilot"] for f in fs)):
        bins[src] = (build(old, src, "old", env), build(new, src, "new", env))
    return bins


def counts(old, new, path):
    fs = forms(path)
    bins = pilots(old, new, fs)
    print("| form | before | after | change | output |")
    print("| --- | ---: | ---: | ---: | --- |")
    worse = []
    for f in fs:
        b, ob = per_element(bins[f["pilot"]][0], f)
        a, oa = per_element(bins[f["pilot"]][1], f)
        bi, ai = b["instructions"], a["instructions"]
        d = 100 * (ai - bi) / bi if bi else 0.0
        same = "same" if shown(ob) == shown(oa) else "**DIFFERENT**"
        if d > 0.3 or same != "same":
            worse.append(f["name"])
        print("| %s | %.2f | %.2f | %+.2f %% | %s |" % (f["name"], bi, ai, d, same), flush=True)
    print("\n%d forms; above +0.3 %% or different: %s" % (len(fs), ", ".join(worse) or "none"))


def cycles(old, new, path, names):
    env_file = os.path.join(ROOT, "bench/build/machine.env")
    pin = [l.split("=", 1)[1].strip() for l in open(env_file) if l.startswith("PIN_CPU=")][0]
    if subprocess.run(["pgrep", "-x", "Plex Transcoder"], capture_output=True).returncode == 0:
        sys.exit("a Plex Transcoder runs: no measure of cycles now")
    fs = [f for f in forms(path) if not names or f["name"] in names]
    bins = pilots(old, new, fs)
    print("| form | before cycles | after cycles | change | before instr. | after instr. |")
    print("| --- | ---: | ---: | ---: | ---: | ---: |")
    for f in fs:
        runs = {0: [], 1: []}
        for _ in range(9):
            for side in (0, 1):
                v, _ = per_element(bins[f["pilot"]][side], f, "cycles:u,instructions:u", pin)
                runs[side].append(v)
        med = lambda side, e: statistics.median(r[e] for r in runs[side])
        b, a = med(0, "cycles"), med(1, "cycles")
        print("| %s | %.2f | %.2f | %+.2f %% | %.2f | %.2f |" % (f["name"], b, a, 100 * (a - b) / b, med(0, "instructions"), med(1, "instructions")), flush=True)


def most_memory(cmd):
    """`ru_maxrss` of the child alone (a process of its own for each)."""
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(r)
        subprocess.run(cmd, stdout=subprocess.DEVNULL, env=dict(os.environ, ISC_THREADS="1"))
        os.write(w, str(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss).encode())
        os._exit(0)
    os.close(w)
    os.waitpid(pid, 0)
    return int(os.read(r, 64).decode())


def memory(new, path):
    fs = forms(path)
    print("| form | KiB at r rounds | KiB at 10 r | grows |")
    print("| --- | ---: | ---: | --- |")
    for f in fs:
        b = build(new, f["pilot"], "new")
        few, many = most_memory([b] + args_of(f, f["r2"])), most_memory([b] + args_of(f, 10 * f["r2"]))
        print("| %s | %d | %d | %s |" % (f["name"], few, many, "**yes**" if many > few + few // 7 + 256 else "no"), flush=True)


def interp(old, new, path):
    fs = forms(path)
    print("| form | before ns | after ns | factor |")
    print("| --- | ---: | ---: | ---: |")
    for f in fs:
        t = []
        for iscc in (old, new):
            p = subprocess.run([iscc, "run", os.path.join(ROOT, f["pilot"])] + args_of(f, max(1, f["r1"] // 10)), capture_output=True, text=True)
            first = p.stdout.split("\n")[0].split()
            t.append(float(first[1]) if p.returncode == 0 and len(first) > 1 else float("nan"))
        print("| %s | %.1f | %.1f | %.2f |" % (f["name"], t[0], t[1], t[1] / t[0] if t[0] else float("nan")), flush=True)


def compile_cost(old, new, programs):
    print("| program | before M instr. | after M instr. | change | before bytes | after bytes |")
    print("| --- | ---: | ---: | ---: | ---: | ---: |")
    for src in programs:
        row = []
        for tag, iscc in (("old", old), ("new", new)):
            out = build(iscc, src, tag)  # (the module of the prelude made first)
            cache = os.path.join(WORK, "cache_" + tag)
            best = min(perf(["env", "ISC_CACHE_DIR=" + cache, iscc, "build", os.path.join(ROOT, src), "-o", out])[0]["instructions"] for _ in range(3))
            row.append((best / 1e6, os.path.getsize(out)))
        (b, sb), (a, sa) = row
        print("| %s | %.1f | %.1f | %+.1f %% | %d | %d |" % (src, b, a, 100 * (a - b) / b, sb, sa), flush=True)


def same(old, new, program, inputs):
    bo, bn = build(old, program, "old"), build(new, program, "new")
    for inp in inputs:
        outs = [subprocess.run([b, inp], capture_output=True).stdout for b in (bo, bn)]
        p = subprocess.run([new, "run", os.path.join(ROOT, program), inp], capture_output=True)
        verdict = "same" if outs[0] == outs[1] else "**DIFFERENT**"
        interp_ok = "same" if p.stdout == outs[0] else "**DIFFERENT** (or not run)"
        print("| %s | compiled: %s | interpreted: %s | %d bytes |" % (inp, verdict, interp_ok, len(outs[0])), flush=True)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    cmd, rest = sys.argv[1], sys.argv[2:]
    if cmd == "counts":
        counts(*rest[:3])
    elif cmd == "cycles":
        cycles(rest[0], rest[1], rest[2], rest[3:])
    elif cmd == "memory":
        memory(*rest[:2])
    elif cmd == "interp":
        interp(*rest[:3])
    elif cmd == "compile":
        compile_cost(rest[0], rest[1], rest[2:])
    elif cmd == "same":
        same(rest[0], rest[1], rest[2], rest[3:])
    else:
        sys.exit(__doc__)
