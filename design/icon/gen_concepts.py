#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate battbar app icon concepts (1024x1024 SVG) on the macOS icon grid:
icon body 824x824 at (100, 100), continuous-corner (superellipse) outline."""
import math

S, M = 1024, 100          # canvas, margin
W = S - 2 * M             # 824


def squircle(x0=M, y0=M, w=W, n=5.0, steps=360):
    """Superellipse |x|^n + |y|^n = 1 scaled to the body: close to Apple's continuous corners."""
    cx, cy, a = x0 + w / 2, y0 + w / 2, w / 2
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, s = math.cos(t), math.sin(t)
        x = cx + a * math.copysign(abs(c) ** (2 / n), c)
        y = cy + a * math.copysign(abs(s) ** (2 / n), s)
        pts.append(f"{x:.2f},{y:.2f}")
    return "M" + " L".join(pts) + " Z"


BODY = squircle()


def svg(defs, content, bg):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{S}" height="{S}" viewBox="0 0 {S} {S}">
  <defs>
    <clipPath id="body"><path d="{BODY}"/></clipPath>
    {defs}
  </defs>
  <path d="{BODY}" fill="{bg}"/>
  <g clip-path="url(#body)">
    {content}
  </g>
</svg>
'''


def pill(x, y, w, h, fill, opacity=1):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{h / 2}" fill="{fill}" opacity="{opacity}"/>'


# A — "Levels": three battery levels, one per device, as pills on a deep blue gradient.
a_defs = '''<linearGradient id="ga" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#2E6BFF"/><stop offset="1" stop-color="#0B2A8C"/></linearGradient>
    <linearGradient id="gloss" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.18"/><stop offset="0.5" stop-color="#fff" stop-opacity="0"/></linearGradient>'''
bars = []
for i, (frac, col) in enumerate([(0.86, "#7CF5B0"), (0.56, "#FFD66B"), (0.26, "#FF7A6B")]):
    y = 300 + i * 150
    bars.append(pill(232, y, 560, 104, "#ffffff", 0.16))
    bars.append(pill(232, y, 560 * frac, 104, col))
a = svg(a_defs, "\n    ".join(bars) + f'\n    <rect x="0" y="0" width="{S}" height="{S}" fill="url(#gloss)"/>', "url(#ga)")

# B — "Menu bar": a light menu bar strip on graphite, holding three small battery glyphs.
b_defs = '''<linearGradient id="gb" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#3A3F4B"/><stop offset="1" stop-color="#16181D"/></linearGradient>'''
glyphs = [f'<rect x="100" y="100" width="824" height="250" fill="#F2F4F8"/>']
for i, (lvl, col) in enumerate([(0.9, "#22C55E"), (0.55, "#F59E0B"), (0.2, "#EF4444")]):
    x = 205 + i * 215
    glyphs.append(f'<rect x="{x}" y="175" width="170" height="100" rx="26" fill="none" stroke="#1F2937" stroke-width="16"/>')
    glyphs.append(f'<rect x="{x + 170}" y="205" width="14" height="40" rx="6" fill="#1F2937"/>')
    glyphs.append(f'<rect x="{x + 16}" y="191" width="{138 * lvl:.0f}" height="68" rx="14" fill="{col}"/>')
glyphs.append('<path d="M512 470 L452 640 H512 L482 800 L612 590 H548 L590 470 Z" fill="#FFD60A"/>')
b = svg(b_defs, "\n    ".join(glyphs), "url(#gb)")

# C — "Signal battery": one large battery with three segments and wireless arcs.
c_defs = '''<linearGradient id="gc" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#FF8A3D"/><stop offset="1" stop-color="#E0337E"/></linearGradient>'''
seg = ['<rect x="210" y="420" width="560" height="300" rx="70" fill="none" stroke="#fff" stroke-width="40"/>',
       '<rect x="770" y="515" width="44" height="110" rx="16" fill="#fff"/>']
for i in range(3):
    seg.append(f'<rect x="{262 + i * 162}" y="472" width="132" height="196" rx="30" fill="#fff" opacity="{[1, 1, 0.35][i]}"/>')
for r, op in [(120, 1), (200, 0.75), (280, 0.5)]:
    seg.append(f'<path d="M {512 - r * 0.7:.1f} {370 - r * 0.45:.1f} A {r} {r} 0 0 1 {512 + r * 0.7:.1f} {370 - r * 0.45:.1f}" '
               f'fill="none" stroke="#fff" stroke-width="34" stroke-linecap="round" opacity="{op}"/>')
c = svg(c_defs, "\n    ".join(seg), "url(#gc)")

for name, content in [("A-levels", a), ("B-menu-bar", b), ("C-signal-battery", c)]:
    with open(f"concepts/{name}.svg", "w") as f:
        f.write(content)
print("ok")
