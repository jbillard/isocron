"""The floor of each program on this machine (DECISIONS D182, D222): the
work its result requires (counted: bench/floor/counts.c, or below), times
the cost of the cheapest way the machine has to do it (measured:
bench/floor/floor.c), never the time of one program. Prints a
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

# Each floor (D222): what the result requires, under the rules of the
# Benchmarks Game (its algorithm: the trees allocated, the spigot, the
# generator, the hash tables of k-mers...), at the cost of the cheapest way
# the machine has to do it, measured by bench/floor/floor.c; never the time
# of a program. A floor that a measured program beats is wrong: flagged.

# binary-trees 21: every node made once (a bump in an arena) and visited
# once through its pointers; the trees of the iterations in parallel, the
# stretch tree and the long-lived one alone. A node at the cheaper of a
# recursion and of a loop with an explicit stack.
def binarytrees():
    small = min(c4("tree_node_depth12"), c4("tree_loop_depth12"))
    large = min(c4("tree_node_depth20"), c4("tree_loop_depth20"))
    seq = ((2 ** 23 - 1) + (2 ** 22 - 1)) * min(c1("tree_node_depth20"), c1("tree_loop_depth20")) * ns
    par = 0.0
    for d in range(4, 22, 2):
        trees, nodes = 2 ** (21 - d + 4), 2 ** (d + 1) - 1
        par += trees * nodes * (small if d <= 14 else large) * ns
    return seq + par / T, "each node made in an arena and visited through its pointers: %.2f ns per node (%.2f in memory)" % (small, large), "measured"
rows.append(("binarytrees", binarytrees()))

# fannkuch-redux 12: every permutation made (the order of the program) and
# flipped until its first element is 0; the flips of different
# permutations are independent: interleaved, 4 permutations per vector
def fannkuch():
    perms, flips = K["perms"], K["flips"]
    f = c4("fannkuch_flip")
    g = c4("fannkuch_next_permutation")
    return (perms * g + flips * f) * ns / T, "%.2e flips (%.3f ns, 4 permutations per vector), %.2e permutations (%.3f ns)" % (flips, f, perms, g), "measured"
rows.append(("fannkuchredux", fannkuch()))

# fasta 25000000: the generator has a period of 139968 values: one period
# computed, and the symbol of each value for the two tables (IUB, homo
# sapiens), at the cost of a number of the generator in lanes and its
# comparisons; the 254 166 745 bytes of output then copied from tables
# (the period of the symbols, the 287 bytes of ALU) in lines of 60 and a
# newline, into blocks that stay in the caches, written to /dev/null
def fasta():
    period = 139968
    make = 3 * period * c4("fasta_number_iub")
    copy = 254166745 * c4("table_rows")
    return (make + copy) * ns / T, "one period of the generator (139968 values, %.3f ns) and its symbols, then 254 MB of lines copied from tables (%.4f ns a byte)" % (c4("fasta_number_iub"), c4("table_rows")), "measured"
rows.append(("fasta", fasta()))

# k-nucleotide: the input read by every thread into new pages, then 7
# frames of 125e6 windows counted: frames 1 to 6 in tables of at most 4096
# counters (one per part of the sequence), 12 and 18 in hash tables of about
# 140 000 keys (2 MiB, random increments)
def knucleotide():
    b = 254166745
    read = b / (c4("file_read_new") * 1e9)
    small = sum(K["kmers_%d" % k][0] for k in (1, 2, 3, 4, 6)) * c4("count_table_4096")
    large = sum(K["kmers_%d" % k][0] for k in (12, 18)) * c4("random_increment_2048KiB")
    return read + (small + large) * ns / T, "read %.3f s (4 threads), 8.75e8 counts: %.3f ns (tables of 4096), %.3f ns (%d and %d keys for 12, 18)" % (read, c4("count_table_4096"), c4("random_increment_2048KiB"), K["kmers_12"][1], K["kmers_18"][1]), "measured"
rows.append(("knucleotide", knucleotide()))

# mandelbrot 16000: the 7 operations of an iteration as written (zr zr, zi
# zi, their difference, + cr, 2 zr, its product by zi, + ci: a fused
# multiply-add rounds once, other bits, D227), the test |z| > 2 every 8
# iterations (one addition: the squares are those of the next iteration;
# after an escape |z| stays finite for more than 8)
def mandelbrot():
    ops = 7 + 1 / 8
    return K["iterations"] * ops * c4("fma_f64") * ns / T, "%.2e iterations x %.2f operations, 8 lanes per cycle" % (K["iterations"], ops), "measured"
rows.append(("mandelbrot", mandelbrot()))

# n-body 50e6 steps, one thread: the chain of a step as written (difference
# 1, square of the distance 3, root, its cube 2, dt divided by it, the
# product by m dx 1, the 4 contributions to a velocity in their order 4,
# the position 2; the latencies of the root and of the division were
# measured with an addition each); 10 roots and divisions per step if the
# divider is the bound instead
def nbody():
    a = c1("add_latency")
    chain = 11 * a + c1("sqrt_latency") + c1("divide_latency")
    through = 10 * (c1("sqrt_f64") + c1("divide_f64"))
    return 50e6 * max(chain, through) * ns, "latency of a step %.1f ns (10 roots and divisions: %.1f ns)" % (chain, through), "estimate"
rows.append(("nbody", nbody()))

# pidigits 10000, one thread: the operations of the spigot on their real
# sizes (simulated here), one pass over the words of the larger operand
# each, the ones that fuse fused (`(acc + 2 num) k2` is two, `(acc - den d)
# 10` two); the digit (num 3 + acc) / den and its test with num 4 from the
# leading words only (a constant number of words); a pass at the cost of
# a word times a word in assembly
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
        d = (num * 3 + acc) // den
        if d != (num * 4 + acc) // den:
            continue
        passes += words(num) + 2 * max(words(acc), words(den))
        num, acc = num * 10, (acc - den * d) * 10
        i += 1
    w = min(c1("mul_word"), c1("mul_word_asm"))
    return passes * w * ns, "%.2e passes over a word (mulx, adc: %.3f ns)" % (passes, w), "measured"
rows.append(("pidigits", pidigits()))

# regex-redux: the input (50.8 MB) read by every thread into new pages; the
# cleaning a copy; 9 counts, each a scan of the text at the cost of the two
# literals of 8 bytes of a pattern found in 64 bytes at once, or of reading
# the text from memory, the larger; 5 replacements, each a copy
def regexredux():
    b = 50.8e6
    read = b / (c4("file_read_new") * 1e9)
    copy = b / (c4("copy") * 1e9)
    scan = max(b * c4("literal_pair_scan") * ns / T, b / (c4("read") * 1e9))
    return read + copy + 9 * scan + 5 * max(copy, scan), "read %.1f ms, 6 copies of 50.8 MB at %.1f GB/s, 9 scans for literals (%.3f ns per byte)" % (read * 1e3, c4("copy"), c4("literal_pair_scan")), "estimate"
rows.append(("regexredux", regexredux()))

# reverse-complement: the input (254 MB) read by every thread into new pages,
# then each sequence turned in place (a load and a store of each line); or,
# the cheaper, the file read twice by blocks that stay in the caches (the
# headers found, then each sequence read from its end, turned, written to
# /dev/null)
def revcomp():
    b = 254166745
    whole = b / (c4("file_read_new") * 1e9) + b / (c4("update_in_place") * 1e9)
    blocks = 2 * b / (c4("file_read_blocks") * 1e9)
    if whole <= blocks:
        return whole, "read into new pages %.1f GB/s, turned in place %.1f GB/s" % (c4("file_read_new"), c4("update_in_place")), "measured"
    return blocks, "the file read twice by blocks in the caches, %.1f GB/s" % c4("file_read_blocks"), "measured"
rows.append(("revcomp", revcomp()))

# spectral-norm 5500: 40 products of the matrix by a vector, 1.21e9
# elements; each one a division, or read from memory where it was stored
# once (8 bytes): the cores divide part of the elements while the memory
# brings the others, in the proportions that end together
def spectralnorm():
    d = 40 * 5500 * 5500
    cd = c4("divide_f64") * ns / T
    cm = 8 / (c4("read") * 1e9)
    f = cd / (cd + cm)
    store = f * 5500 * 5500 * (cd + 8 / (c4("new_pages") * 1e9))
    mixed = d * cd * cm / (cd + cm) + store
    return min(d * cd, mixed), "%.2e elements: %.0f %% divided (%.3f ns per core), %.0f %% read from memory, stored once" % (d, 100 * (1 - f), c4("divide_f64"), 100 * f), "measured"
rows.append(("spectralnorm", spectralnorm()))

print("| benchmark | floor | model | how | Isocron | Isocron / floor | fastest other | its / floor |")
print("| --- | ---: | --- | --- | ---: | ---: | --- | ---: |")
beaten = []
for name, (f, model, how) in rows:
    iso = ISO[name]
    prog, ms = OTHER[name]
    if min(iso, ms / 1000) < f:
        beaten.append(name)
    print("| %s | %.3f s | %s | %s | %.3f s | %.1f | %s %.3f s | %.1f |" % (name, f, model, how, iso, iso / f, prog, ms / 1000, ms / 1000 / f))
if beaten:
    # a floor below a measured time is wrong: its model misses a way to do it
    print("\n**Floors beaten by a measured program (wrong models)**: " + ", ".join(beaten))
    sys.stderr.write("floors beaten: %s\n" % ", ".join(beaten))
