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
    """A Mermaid bar chart (rendered by GitHub), with a line (a reference).
    Horizontal: one row a label, the names of the ten programs side by side
    under vertical bars overlap."""
    out = ["```mermaid", "xychart-beta horizontal", '    title "%s"' % title,
           "    x-axis [%s]" % ", ".join('"%s"' % l for l in labels),
           '    y-axis "%s" 0 --> %s' % (axis, top),
           "    bar [%s]" % ", ".join("%.2f" % v for v in values)]
    if line is not None:
        out.append("    line [%s]" % ", ".join("%.2f" % v for v in line))
    out.append("```")
    return "\n".join(out)


def ratio_svg(title, subtitle, items, reference):
    """A chart of ratios as an SVG image (the same on every viewer, unlike a
    Mermaid chart): one row a label, its bar, its value; a dashed line at 1,
    named `reference`. The colors of `status` (green below 0.95, amber to
    1.05, red beyond); the texts gray, readable on a light and a dark page."""
    row, left, plot, top = 30, 190, 470, 74
    vmax = max(1.2, max(v for _, v in items) * 1.08)
    x = lambda v: left + v / vmax * plot
    h = top + row * len(items) + 34
    w = left + plot + 60
    ink, faint = "#8b949e", "#8b949e55"
    out = ['<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %d %d" '
           'font-family="-apple-system, Segoe UI, Helvetica, Arial, sans-serif">' % (w, h, w, h),
           '<text x="0" y="22" font-size="17" font-weight="600" fill="%s">%s</text>' % (ink, title),
           '<text x="0" y="44" font-size="13" fill="%s">%s</text>' % (ink, subtitle)]
    for t in [i * 0.25 for i in range(int(vmax / 0.25) + 1)]:
        out.append('<line x1="%.1f" y1="%d" x2="%.1f" y2="%d" stroke="%s"/>' % (x(t), top - 6, x(t), h - 30, faint))
        out.append('<text x="%.1f" y="%d" font-size="11" fill="%s" text-anchor="middle">%g</text>' % (x(t), h - 14, ink, t))
    for i, (label, v) in enumerate(items):
        y = top + i * row
        c = "#3fb950" if v < 0.95 else "#d29922" if v <= 1.05 else "#f85149"
        out.append('<text x="%d" y="%d" font-size="13" fill="%s" text-anchor="end">%s</text>' % (left - 10, y + 14, ink, label))
        out.append('<rect x="%d" y="%d" width="%.1f" height="18" rx="3" fill="%s"/>' % (left, y, max(1.5, x(v) - left), c))
        # the value inside the end of its bar (none crosses the line at 1),
        # after it when the bar is too short to hold it
        if x(v) - left > 48:
            out.append('<text x="%.1f" y="%d" font-size="12" font-weight="600" fill="#ffffff" text-anchor="end">%.2f</text>'
                       % (x(v) - 6, y + 13, v))
        else:
            out.append('<text x="%.1f" y="%d" font-size="12" fill="%s">%.2f</text>' % (x(v) + 6, y + 13, ink, v))
    out.append('<line x1="%.1f" y1="%d" x2="%.1f" y2="%d" stroke="%s" stroke-width="1.5" stroke-dasharray="5 4"/>'
               % (x(1), top - 10, x(1), h - 30, ink))
    out.append('<text x="%.1f" y="%d" font-size="11" fill="%s" text-anchor="middle">%s</text>' % (x(1), top - 16, ink, reference))
    out.append("</svg>")
    return "\n".join(out) + "\n"


VERDICTS = {"beats": "🟢", "equals": "🟡", "slower": "🔴"}


def verdicts(text):
    """The verdicts of SPEC section 14 in the cells of tables, with their
    mark: `| **beats** |` -> `| 🟢 **beats** |` (also without bold)."""
    for word, mark in VERDICTS.items():
        text = re.sub(r"\| (\*\*)?%s(\*\*)? \|" % word, lambda m: "| %s %s%s%s |" % (mark, m.group(1) or "", word, m.group(2) or ""), text)
        # a cell that holds two (`| **beats** / equals |`)
    return text
