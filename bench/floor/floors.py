"""The floor of each program on this machine (DECISIONS D182): the work it
cannot avoid (counted: bench/floor/counts.c, or below), times the cost of
the resource that limits it (measured: bench/floor/floor.c). Prints a
Markdown table against the times of Isocron and of the fastest program of
another language.
  floors.py CONSTANTS COUNTS ISOCRON_TSV OTHERS_TSV
CONSTANTS: the output of bench/floor/run.sh; COUNTS: of counts.c;
ISOCRON_TSV: `benchmark seconds` lines; OTHERS_TSV: results/benchmarks_game.tsv."""
import re, sys

def constants(path):
    c, threads = {}, None
    for line in open(path):
        m = re.match(r"# (\d+) thread", line)
        if m:
            threads = int(m.group(1))
            continue
        p = line.split()
        if len(p) >= 2:
            c[(p[0], threads)] = float(p[1])
    return c

C = constants(sys.argv[1])
K = {}
for line in open(sys.argv[2]):
    p = line.split()
    if p[0] == "fannkuch":
        K["flips"], K["perms"] = int(p[5]), int(p[3])
    elif p[0] == "mandelbrot":
        K["iterations"] = int(p[5])
    elif p[0] == "kmers" and p[1].startswith("k="):
        K["kmers_" + p[1][2:]] = (int(p[3]), int(p[5]))
ISO = {l.split()[0]: float(l.split()[1]) for l in open(sys.argv[3]) if l.strip()}
OTHER = {}
for l in open(sys.argv[4]):
    mode, bench, prog, ms = l.split()
    if prog != "isocron" and ms.isdigit():
        if bench not in OTHER or int(ms) < OTHER[bench][1]:
            OTHER[bench] = (prog, int(ms))

ns = 1e-9
T = 4
c4 = lambda k: C[(k, 4)]
c1 = lambda k: C[(k, 1)]
rows = []

# binary-trees 21: every node made once and visited once; the trees of the
# iterations in parallel, the stretch tree and the long-lived one alone
def binarytrees():
    small, large = c4("tree_node_depth12"), c4("tree_node_depth20")
    seq = ((2 ** 23 - 1) + (2 ** 22 - 1)) * c1("tree_node_depth20") * ns
    par = 0.0
    for d in range(4, 22, 2):
        trees, nodes = 2 ** (21 - d + 4), 2 ** (d + 1) - 1
        par += trees * nodes * (small if d <= 14 else large) * ns
    return seq + par / T, "make and visit each node (C, an arena): %.0f ns per node" % small, "measured"
rows.append(("binarytrees", binarytrees()))

# fannkuch-redux 12: the flips of a permutation depend on each other (the
# next flip reads the first element): one L1 load per flip at least
def fannkuch():
    return K["flips"] * c1("l1_latency") * ns / T, "%.2e flips, each after a dependent load (%.2f ns)" % (K["flips"], c1("l1_latency")), "estimate"
rows.append(("fannkuchredux", fannkuch()))

# fasta 25000000: 254 166 745 bytes of output written into new memory; each
# byte a lookup in a table of one period of the generator
def fasta():
    b = 254166745
    return b / (c4("new_pages") * 1e9) + b / (c4("read") * 1e9), "254 MB written into new pages, read back once", "estimate"
rows.append(("fasta", fasta()))

# k-nucleotide: the input read (as reverse-complement), then 7 frames of
# 125e6 windows counted: frames 1 to 6 in tables in L2, 12 and 18 in tables
# of about 140 000 keys (in L3)
def knucleotide():
    read = 0.056
    small = sum(K["kmers_%d" % k][0] for k in (1, 2, 3, 4, 6)) * c4("random_increment_256KiB")
    large = sum(K["kmers_%d" % k][0] for k in (12, 18)) * c4("random_increment_2048KiB")
    return read + (small + large) * ns / T, "read 0.056 s, 8.75e8 counts (%d and %d keys for 12, 18)" % (K["kmers_12"][1], K["kmers_18"][1]), "measured"
rows.append(("knucleotide", knucleotide()))

# mandelbrot 16000: 7 floating-point operations per iteration of a pixel
def mandelbrot():
    return K["iterations"] * 7 * c4("fma_f64") * ns / T, "%.2e iterations x 7 operations, 8 lanes per cycle" % K["iterations"], "measured"
rows.append(("mandelbrot", mandelbrot()))

# n-body 50e6 steps, one thread: the chain of each step (positions,
# distance, square root, cube, division, 4 updates of a velocity, position)
def nbody():
    a = c1("add_latency")
    chain = a + 3 * a + (c1("sqrt_latency") - a) + 2 * a + (c1("divide_latency") - a) + a + 4 * a + 2 * a
    through = 10 * (c1("sqrt_f64") + c1("divide_f64"))
    return 50e6 * max(chain, through) * ns, "latency of a step %.1f ns (10 roots and divisions: %.1f ns)" % (chain, through), "estimate"
rows.append(("nbody", nbody()))

# pidigits 10000, one thread: the operations of the program on their real
# sizes (simulated here), at least one pass over the words of the larger
# operand each, the ones that fuse fused (`(acc + 2 num) k2` is two, `acc -
# den d` one); a division whose quotient is one digit, one pass
def pidigits():
    words = lambda x: max(1, (abs(x).bit_length() + 63) // 64)
    n, i, k = 10000, 0, 1
    num, acc, den = 1, 0, 1
    passes = 0
    while i < n:
        k2 = 2 * k + 1
        passes += words(num) + 2 * max(words(acc), words(num)) + words(den)
        num, acc, den = num * k, (acc + num * 2) * k2, den * k2
        k += 1
        if num > acc:
            continue
        passes += max(words(num), words(acc)) + words(den)
        d = (num * 3 + acc) // den
        passes += words(num) + words(den)
        if d != (num * 4 + acc) // den:
            continue
        passes += words(num) + 2 * words(acc)
        num, acc = num * 10, (acc - den * d) * 10
        i += 1
    return passes * c1("mul_word") * ns, "%.2e passes over a word (mulx, adc: %.2f ns)" % (passes, c1("mul_word")), "measured"
rows.append(("pidigits", pidigits()))

# regex-redux: each pass of an automaton reads one byte per dependent load
# from L1; 1 cleaning pass, 9 counts, 5 replacements over about 50 MB
def regexredux():
    b = 50.8e6
    return 15 * b * c1("l1_latency") * ns / T, "15 passes over 50.8 MB, one dependent load per byte", "estimate"
rows.append(("regexredux", regexredux()))

# reverse-complement: the input read (readStdin alone, D179), one pass of
# reading and writing over 254 MB
def revcomp():
    b = 254166745
    return 0.056 + 2 * b / (c4("read") * 1e9), "read 0.056 s, then 2 x 254 MB at %.1f GB/s" % c4("read"), "measured"
rows.append(("revcomp", revcomp()))

# spectral-norm 5500: 40 products of the matrix by a vector, a division per
# element
def spectralnorm():
    d = 40 * 5500 * 5500
    return d * c4("divide_f64") * ns / T, "%.2e divisions at %.3f ns per core" % (d, c4("divide_f64")), "measured"
rows.append(("spectralnorm", spectralnorm()))

print("| benchmark | floor | model | how | Isocron | Isocron / floor | fastest other | its / floor |")
print("| --- | ---: | --- | --- | ---: | ---: | --- | ---: |")
for name, (f, model, how) in rows:
    iso = ISO[name]
    prog, ms = OTHER[name]
    print("| %s | %.3f s | %s | %s | %.3f s | %.1f | %s %.3f s | %.1f |" % (name, f, model, how, iso, iso / f, prog, ms / 1000, ms / 1000 / f))
