#!/usr/bin/env python3
"""The audit of every form of the language against the machine (TODO, the
remark of the author of 2026-11-04): each group of forms is a program in
Isocron (`<group>.isc`) and the same work in C (`<group>.c`, the floor),
each printing one line per form, `name nanoseconds checksum`. Both are
built, run RUNS times on one CPU of the measurements (`bench/machine.sh
apply` first, at fixed frequency), the median of each form kept; the
checksums must agree. Writes results/primitives.tsv and
results/primitives.md, the forms ranked by Isocron over its floor.

    python3 bench/primitives/run.py [--runs 5] [--rounds 200] [groups...]
"""
import argparse
import os
import statistics
import socket
import subprocess
import time
import sys

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(os.path.dirname(here))
build = os.path.join(here, "build")


def bench_cpu():
    """The CPU of the measurements of one thread (`machine.sh apply`)."""
    env = os.path.join(root, "bench", "build", "machine.env")
    for line in open(env):
        if line.startswith("PIN_CPU="):
            return line.split("=", 1)[1].strip()
    sys.exit("bench/machine.sh apply first")


def run(cmd, cpu):
    """`cmd` on the CPU `cpu` alone, in the slice of the measurements; its
    lines as {name: (ns, checksum)}."""
    full = ["sudo", "-n", "systemd-run", "--quiet", "--scope", "--slice=isc-bench", "-p", "AllowedCPUs=" + cpu,
            "--uid=%d" % os.getuid(), "--gid=%d" % os.getgid(), "-E", "ISC_THREADS=1"] + cmd
    out = subprocess.run(full, check=True, stdout=subprocess.PIPE, text=True).stdout
    forms = {}
    for line in out.splitlines():
        name, ns, check = line.split()
        forms[name] = (float(ns), check)
    return forms


def tls_server():
    """For a group that declares `// bench-server: tls` (D343): an authority
    and a certificate for `localhost` made in `build/tls`, `openssl s_server`
    serving its files (`-WWW`: `hello.txt`, `64k.bin`) on a free port; the
    server and the arguments that the benchmark takes after its rounds (the
    port, the authority)."""
    d = os.path.join(build, "tls")
    os.makedirs(os.path.join(d, "www"), exist_ok=True)
    ec = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"]
    with open(os.path.join(d, "ext.cnf"), "w") as f:
        f.write("subjectAltName=DNS:localhost\n")
    for cmd in (["req", "-x509"] + ec + ["-keyout", "ca.key", "-out", "ca.pem", "-days", "2", "-subj", "/CN=bench ca"],
                ["req"] + ec + ["-keyout", "server.key", "-out", "server.csr", "-subj", "/CN=localhost"],
                ["x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "server.pem", "-days", "2", "-extfile", "ext.cnf"]):
        subprocess.run(["openssl"] + cmd, cwd=d, check=True, capture_output=True)
    with open(os.path.join(d, "www", "hello.txt"), "w") as f:
        f.write("hello, world!")
    with open(os.path.join(d, "www", "64k.bin"), "wb") as f:
        f.write(bytes(i % 251 for i in range(65536)))
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    server = subprocess.Popen(["openssl", "s_server", "-quiet", "-accept", str(port), "-cert", "../server.pem", "-key", "../server.key", "-WWW"],
                              cwd=os.path.join(d, "www"), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(500):
        try:
            socket.create_connection(("127.0.0.1", port), timeout=1).close()
            break
        except OSError:
            time.sleep(0.01)
    return server, [str(port), os.path.join(d, "ca.pem")]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--rounds", type=int, default=200)
    ap.add_argument("groups", nargs="*")
    a = ap.parse_args()
    groups = a.groups or sorted(f[:-4] for f in os.listdir(here) if f.endswith(".isc"))
    os.makedirs(build, exist_ok=True)
    iscc = os.path.join(root, "target", "release", "iscc")
    cpu = bench_cpu()
    rows = []
    for g in groups:
        isc_bin, c_bin = os.path.join(build, g), os.path.join(build, g + "_c")
        subprocess.run([iscc, "build", os.path.join(here, g + ".isc"), "-o", isc_bin], check=True)
        # the floor of TLS calls OpenSSL itself (D343)
        libs = ["-l:libssl.so.3", "-l:libcrypto.so.3"] if "// bench-server: tls" in open(os.path.join(here, g + ".isc")).read() else []
        subprocess.run(["gcc", "-O3", "-march=native", os.path.join(here, g + ".c"), "-o", c_bin] + libs, check=True)
        times = {"isc": {}, "c": {}}
        checks = {"isc": {}, "c": {}}
        server, extra = None, []
        if "// bench-server: tls" in open(os.path.join(here, g + ".isc")).read():
            server, extra = tls_server()
        try:
            for _ in range(a.runs):
                for kind, b in (("isc", isc_bin), ("c", c_bin)):
                    for name, (ns, check) in run([b, str(a.rounds)] + extra, cpu).items():
                        times[kind].setdefault(name, []).append(ns)
                        checks[kind][name] = check
        finally:
            if server:
                server.kill()
                server.wait()
        for name in times["isc"]:
            if name not in times["c"]:
                sys.exit("%s: no floor for %s" % (g, name))
            if checks["isc"][name] != checks["c"][name]:
                sys.exit("%s: %s computes %s, its floor %s" % (g, name, checks["isc"][name], checks["c"][name]))
            isc, floor = statistics.median(times["isc"][name]), statistics.median(times["c"][name])
            rows.append((g, name, isc, floor))
    # the rows of the groups not measured this time kept (D392: measuring
    # some groups wrote a page of those alone)
    old = os.path.join(root, "results", "primitives.tsv")
    if os.path.exists(old):
        for line in open(old):
            g, name, isc, floor = line.rstrip("\n").split("\t")
            if g not in groups:
                rows.append((g, name, float(isc), float(floor)))
    rows.sort(key=lambda r: -r[2] / r[3])
    with open(os.path.join(root, "results", "primitives.tsv"), "w") as f:
        for g, name, isc, floor in rows:
            f.write("%s\t%s\t%.3f\t%.3f\n" % (g, name, isc, floor))
    with open(os.path.join(root, "results", "primitives.md"), "w") as f:
        f.write("# Every form against the machine\n\n")
        f.write("Each form of the language measured in Isocron and as the same work in C\n")
        f.write("written the cheapest way known (its floor), on one CPU at fixed frequency,\n")
        f.write("the median of %d runs of %d rounds (`bench/primitives/run.py`); the\n" % (a.runs, a.rounds))
        f.write("nanoseconds of one call, ranked by Isocron over its floor.\n\n")
        f.write("| group | form | Isocron (ns) | floor (ns) | Isocron / floor |\n")
        f.write("| :-- | :-- | --: | --: | --: |\n")
        for g, name, isc, floor in rows:
            f.write("| %s | `%s` | %.2f | %.2f | %.1f |\n" % (g, name, isc, floor, isc / floor))
    for g, name, isc, floor in rows:
        print("%-10s %-16s %10.2f %10.2f %6.1f" % (g, name, isc, floor, isc / floor))


main()
