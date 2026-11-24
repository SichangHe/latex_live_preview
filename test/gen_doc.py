"""Writes a long test document whose words are all unique, so a click maps to one spot.

Usage: gen_doc.py OUT.tex N_SECTIONS
"""
import random
import sys


def word(n):
    s = ""
    n += 26 * 26
    while n:
        n, r = divmod(n, 26)
        s = chr(97 + r) + s
    return "q" + s


def main():
    out_path, n_sections = sys.argv[1], int(sys.argv[2])
    rng = random.Random(1)
    n = 0
    lines = [r"\documentclass{article}", r"\usepackage{amsmath,hyperref}", r"\begin{document}"]
    for s in range(n_sections):
        lines.append(r"\section{Section %s}\label{sec%d}" % (word(n), s))
        n += 1
        for _ in range(4):
            ws = []
            for _ in range(rng.randint(60, 120)):
                w = word(n)
                n += 1
                k = rng.random()
                ws.append(r"\emph{%s}" % w if k < 0.05 else r"\textbf{%s}" % w if k < 0.08 else w)
            lines.append(" ".join(ws[:40]))
            lines.append(" ".join(ws[40:]) + r" See Section~\ref{sec%d}." % max(0, s - 1))
            lines.append("")
        lines.append(r"\begin{equation} E_{%d} = \sum_{i=0}^{n} x_i^2 \end{equation}" % s)
        lines.append("")
    lines.append(r"\end{document}")
    with open(out_path, "w") as f:
        f.write("\n".join(lines) + "\n")


main()
