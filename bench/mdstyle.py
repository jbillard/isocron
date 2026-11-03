"""The look of the generated Markdown on GitHub (D235): GitHub keeps no
color of HTML or CSS, but renders the colors of math (`$\\color{...}{...}$`,
also in tables), emoji, alerts (`> [!NOTE]`) and Mermaid charts."""

import re

NAMES = {
    "binarytrees": "binary-trees",
    "fannkuchredux": "fannkuch-redux",
    "fasta": "fasta",
    "knucleotide": "k-nucleotide",
    "mandelbrot": "mandelbrot",
    "nbody": "n-body",
    "pidigits": "pidigits",
    "regexredux": "regex-redux",
    "revcomp": "reverse-complement",
    "spectralnorm": "spectral-norm",
}


def name(bench):
    return NAMES.get(bench, bench)


def color(text, c, bold=False):
    """`text` in the color `c` (a name of LaTeX: green, red, orange...)."""
    t = text.replace("%", "\\%")
    if bold:
        t = "\\textbf{%s}" % t
    return "$\\color{%s}{%s}$" % (c, t)


def vs_fastest(r):
    """Isocron over the fastest other: green when faster, orange within
    5 %, red behind."""
    s = "%.2f" % r
    if r < 0.95:
        return color(s, "green", True)
    if r <= 1.05:
        return color(s, "orange", True)
    return color(s, "red", True)


def status(r):
    return "🟢" if r < 0.95 else "🟡" if r <= 1.05 else "🔴"


def vs_floor(g):
    """A time over its floor: green up to 1.5, orange up to 3, red beyond."""
    s = "%.1f" % g
    if g <= 1.5:
        return color(s, "green")
    if g <= 3:
        return color(s, "orange")
    return color(s, "red")


LEGEND = ("🟢 Isocron the fastest · 🟡 within 5 % of the fastest · 🔴 behind. "
          "Over the floor: $\\color{green}{\\text{up to 1.5}}$, $\\color{orange}{\\text{up to 3}}$, "
          "$\\color{red}{\\text{beyond}}$.")


def bar_chart(title, labels, values, axis, top, line=None):
    """A Mermaid bar chart (rendered by GitHub), with a line (a reference)."""
    out = ["```mermaid", "xychart-beta", '    title "%s"' % title,
           "    x-axis [%s]" % ", ".join('"%s"' % l for l in labels),
           '    y-axis "%s" 0 --> %s' % (axis, top),
           "    bar [%s]" % ", ".join("%.2f" % v for v in values)]
    if line is not None:
        out.append("    line [%s]" % ", ".join("%.2f" % v for v in line))
    out.append("```")
    return "\n".join(out)


VERDICTS = {"beats": "🟢", "equals": "🟡", "slower": "🔴"}


def verdicts(text):
    """The verdicts of SPEC section 14 in the cells of tables, with their
    mark: `| **beats** |` -> `| 🟢 **beats** |` (also without bold)."""
    for word, mark in VERDICTS.items():
        text = re.sub(r"\| (\*\*)?%s(\*\*)? \|" % word, lambda m: "| %s %s%s%s |" % (mark, m.group(1) or "", word, m.group(2) or ""), text)
        # a cell that holds two (`| **beats** / equals |`)
    return text
