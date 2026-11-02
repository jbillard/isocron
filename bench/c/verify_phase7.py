#!/usr/bin/env python3
"""Checks the C programs of the kernels of phase 7 (bench/build_phase7_c.sh)
against Isocron.

    python3 bench/c/verify_phase7.py          # the table of bench/c/PHASE7.md
    python3 bench/c/verify_phase7.py --edge   # also synthetic edge cases

Every program runs with reps = 1 on bench/data/small and bench/data/large; its
output is compared with Isocron's (bench/data/ref/<size>/<kernel>.out): either
identical byte for byte, or the largest relative difference. With --edge,
synthetic inputs (block boundaries of the kept elements, signed zeros, NaN,
wrapping i64 sums, odd sizes, cols > 4096) are run through bench/bin/isc_<k>
and every opt build (and the exact idiomatic prefix_sum), with 1, 2, 3, 5
and 16 threads for the OpenMP builds; they must be identical. The running sum
of comparison B (moving_average_b, and Isocron's isc_moving_average_b when it
is in bench/bin) is compared with the exact window sums: the largest absolute
and relative differences."""
import array, math, os, random, struct, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN, DATA, REF = f"{ROOT}/bench/bin", f"{ROOT}/bench/data", f"{ROOT}/bench/data/ref"
INPUTS = {"moving_average": ["a"], "filter_sum": ["a"], "prefix_sum": ["i"], "matvec": ["m", "v"]}
FLOAT = {"moving_average", "filter_sum", "matvec"}


def load(path):
    a = array.array("d")
    with open(path, "rb") as f:
        a.frombytes(f.read())
    return a


def run(prog, args, out, threads=1):
    if os.path.exists(out):
        os.remove(out)
    env = dict(os.environ, OMP_NUM_THREADS=str(threads))
    subprocess.run([f"{BIN}/{prog}", *args, out, "1"], env=env, check=True, stdout=subprocess.DEVNULL)


def same_bytes(a, b):
    # not filecmp.cmp: its cache keys on (size, mtime), and the outputs
    # rewritten to one path within a clock tick would get a stale verdict
    with open(a, "rb") as f, open(b, "rb") as g:
        return f.read() == g.read()


def compare(out, ref):
    if same_bytes(out, ref):
        return "identical"
    c, r = load(out), load(ref)
    if len(c) != len(r):
        return f"LENGTH {len(c)} != {len(r)}"
    worst, ndiff = 0.0, 0
    for x, y in zip(c, r):
        if x != y:
            ndiff += 1
            worst = max(worst, abs(x - y) / abs(y) if y != 0 else math.inf)
    return f"{worst:.2e} ({ndiff}/{len(r)} values differ)"


def reference_table(tmp):
    print("| kernel | size | program | result |\n| --- | --- | --- | --- |")
    for k, ins in INPUTS.items():
        progs = []
        for v in ["opt", "idiomatic"]:
            progs += [(f"c-gcc_{k}_{v}", 1), (f"c-clang_{k}_{v}", 1),
                      (f"c-gcc-omp_{k}_{v}", 1), (f"c-gcc-omp_{k}_{v}", 4)]
        if k in FLOAT:
            progs += [(f"c-gcc-reassoc_{k}_idiomatic", 1), (f"c-clang-reassoc_{k}_idiomatic", 1)]
        for size in ["small", "large"]:
            for prog, t in progs:
                out = f"{tmp}/out.f64"
                run(prog, [f"{DATA}/{size}/{i}.f64" for i in ins], out, t)
                name = prog + (f" T={t}" if "-omp" in prog else "")
                print(f"| {k} | {size} | {name} | {compare(out, f'{REF}/{size}/{k}.out')} |", flush=True)


def comparison_b(tmp):
    print("\n| program | size | largest absolute difference | largest relative difference |")
    print("| --- | --- | --- | --- |")
    progs = [("c-gcc_moving_average_b_idiomatic", 1), ("c-clang_moving_average_b_idiomatic", 1),
             ("c-gcc-omp_moving_average_b_idiomatic", 1), ("c-gcc-omp_moving_average_b_idiomatic", 4)]
    if os.path.exists(f"{BIN}/isc_moving_average_b"):
        progs.append(("isc_moving_average_b", 1))
    for size in ["small", "large"]:
        r = load(f"{REF}/{size}/moving_average.out")
        for prog, t in progs:
            out = f"{tmp}/out.f64"
            run(prog, [f"{DATA}/{size}/a.f64"], out, t)
            c = load(out)
            assert len(c) == len(r), (prog, len(c), len(r))
            dabs = max(abs(x - y) for x, y in zip(c, r))
            drel = max(abs(x - y) / abs(y) if y != 0 else (0.0 if x == y else math.inf) for x, y in zip(c, r))
            name = prog + (f" T={t}" if "-omp" in prog else "")
            print(f"| {name} | {size} | {dabs:.2e} | {drel:.2e} |", flush=True)


def write(path, xs):
    with open(path, "wb") as f:
        f.write(struct.pack(f"<{len(xs)}d", *xs))


def edge_inputs(d):
    """[(kernel, [input paths])]"""
    rnd = random.Random(7)
    wild = lambda: rnd.choice([-1, 1]) * 10 ** rnd.uniform(-8, 8)  # order-sensitive sums
    cases = []
    def add(k, name, *arrays):
        paths = []
        for j, xs in enumerate(arrays):
            paths.append(f"{d}/{name}_{j}.f64")
            write(paths[-1], xs)
        cases.append((k, paths))
    for kept in [0, 1, 31, 32, 33, 4095, 4096, 4097, 8191, 8192, 8193, 12305, 40000]:
        xs = []
        for _ in range(kept):
            for _ in range(rnd.randint(0, 3)):
                xs.append(rnd.choice([-abs(wild()), -0.0, 0.0, math.nan, -math.inf]))
            xs.append(abs(wild()))
        add("filter_sum", f"fs{kept}", xs + [-1.0] * rnd.randint(0, 50))
    xs = [-1.0] * 200000
    for i in rnd.sample(range(200000), 5000):
        xs[i] = abs(wild())
    add("filter_sum", "fs_sparse", xs)
    add("filter_sum", "fs_allpos", [abs(wild()) for _ in range(100003)])
    add("filter_sum", "fs_empty", [])
    for n in [64, 65, 100, 1087, 1088, 1089, 2111, 5000]:
        add("moving_average", f"ma{n}", [wild() for _ in range(n)])
    add("moving_average", "ma_negzero", [-0.0] * 200 + [wild() for _ in range(100)] + [-0.0] * 100)
    for n in [0, 1, 7, 8, 9, 17, 1000003]:
        add("prefix_sum", f"ps{n}", [float(rnd.randint(-1000, 1000)) for _ in range(n)])
    add("prefix_sum", "ps_wrap", [float(rnd.choice([1, -1]) * rnd.randint(2**52, 2**53)) if i % 3
                                  else 2.0**53 for i in range(5000)])
    for r, c in [(3, 5), (7, 31), (5, 32), (9, 100), (3, 4096), (2, 5000), (1, 9000)]:
        add("matvec", f"mv{r}x{c}", [wild() for _ in range(r * c)], [wild() for _ in range(c)])
    return cases


def edge(tmp):
    checks, failures = 0, []
    for k, ins in edge_inputs(tmp):
        ref = f"{tmp}/ref.f64"
        run(f"isc_{k}", ins, ref)
        progs = [(f"c-gcc_{k}_opt", 1), (f"c-clang_{k}_opt", 1)]
        progs += [(f"c-gcc-omp_{k}_opt", t) for t in [1, 2, 3, 5, 16]]
        if k == "prefix_sum":
            progs += [(f"c-gcc_{k}_idiomatic", 1), (f"c-clang_{k}_idiomatic", 1)]
            progs += [(f"c-gcc-omp_{k}_idiomatic", t) for t in [1, 2, 3, 5, 16]]
        for prog, t in progs:
            out = f"{tmp}/out.f64"
            checks += 1
            try:
                run(prog, ins, out, t)
                if not same_bytes(out, ref):
                    failures.append(f"{prog} T={t} {os.path.basename(ins[0])}: differs")
            except subprocess.CalledProcessError as e:
                failures.append(f"{prog} T={t} {os.path.basename(ins[0])}: exit {e.returncode}")
    print(f"\nedge cases: {checks} checks, {len(failures)} failures")
    for f in failures:
        print("  " + f)
    return not failures


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as tmp:
        reference_table(tmp)
        comparison_b(tmp)
        ok = edge(tmp) if "--edge" in sys.argv[1:] else True
    sys.exit(0 if ok else 1)
