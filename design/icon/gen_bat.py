#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Bat icon ("battbar") in the C4-a palette, on the macOS icon grid.

The bat silhouette is built here from simple geometry (ears, arched wing tops,
three scallops per wing), mirrored around x = 512. Writes the concepts
concepts/D*.svg (+ 36 px menu bar glyphs concepts/D*-menubar.svg) and the chosen
design D1 "Echo" as AppIcon.svg and MenuBarIcon.svg.
Build the app resources afterwards with scripts/build-icon.sh."""
from gen_concepts import S, BODY

BG_TOP, BG_BOTTOM, EDGE = "#FFFFFF", "#F0F1F8", "#DADDEB"
G0, G1 = "#7A93F7", "#9673E6"
PALE = "#DCDDF8"


def bat_path(cx=512.0, cy=545.0, span=1.0, height=1.0, ears=True, body_drop=1.0):
    """Closed bat silhouette. Right half is defined relative to the centre, then mirrored.
    Segments are (kind, control, end) with kind 'L' or 'Q'; coordinates as (dx, dy)."""
    sx, sy = span, height
    start = (0, -95)                       # dip between the ears
    right = []
    if ears:
        right += [("L", None, (22, -158)), ("L", None, (44, -88))]   # ear tip, outer ear base
    else:
        right += [("Q", (30, -100), (44, -88))]
    right += [
        ("Q", (64, -80), (80, -62)),                         # shoulder
        ("Q", (210, -175), (372, -150)),                     # wing top edge to the tip
        ("Q", (290, -85), (262, 40)),                        # scallop 1
        ("Q", (205, -15), (160, 62)),                        # scallop 2
        ("Q", (112, 10), (66, 70)),                          # scallop 3
        ("Q", (50, 120 * body_drop), (0, 150 * body_drop)),  # body to the tail
    ]

    def p(d):
        return (cx + d[0] * sx, cy + d[1] * sy)

    def m(d):
        return (cx - d[0] * sx, cy + d[1] * sy)

    out = [f"M {p(start)[0]:.1f} {p(start)[1]:.1f}"]
    for kind, c, e in right:
        if kind == "L":
            out.append(f"L {p(e)[0]:.1f} {p(e)[1]:.1f}")
        else:
            out.append(f"Q {p(c)[0]:.1f} {p(c)[1]:.1f} {p(e)[0]:.1f} {p(e)[1]:.1f}")
    # Left half: the right half mirrored and walked backwards (tail -> dip).
    pts = [start] + [e for _, _, e in right]
    for i in range(len(right) - 1, -1, -1):
        kind, c, _ = right[i]
        e = pts[i]
        if kind == "L":
            out.append(f"L {m(e)[0]:.1f} {m(e)[1]:.1f}")
        else:
            out.append(f"Q {m(c)[0]:.1f} {m(c)[1]:.1f} {m(e)[0]:.1f} {m(e)[1]:.1f}")
    out.append("Z")
    return " ".join(out)


def icon(name, content, extra_defs=""):
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{S}" height="{S}" viewBox="0 0 {S} {S}">
  <defs>
    <clipPath id="body"><path d="{BODY}"/></clipPath>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{BG_TOP}"/><stop offset="1" stop-color="{BG_BOTTOM}"/></linearGradient>
    <linearGradient id="p" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{G0}"/><stop offset="1" stop-color="{G1}"/></linearGradient>
    {extra_defs}
  </defs>
  <path d="{BODY}" fill="url(#bg)"/>
  <g clip-path="url(#body)">
{content}
  </g>
  <path d="{BODY}" fill="none" stroke="{EDGE}" stroke-width="4"/>
</svg>
'''
    open(f"concepts/{name}.svg", "w").write(svg)


def glyph(name, content):
    """36 px menu bar template (black + alpha), viewBox in icon units for reuse."""
    open(f"concepts/{name}-menubar.svg", "w").write(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="36" height="36" viewBox="0 0 1024 1024">{content}</svg>\n')


def arcs(cx, top, radii=(120, 200), ops=(1, 0.55), width=40, color="url(#p)"):
    s = []
    for r, op in zip(radii, ops):
        x0, y0 = cx - r * 0.7, top - r * 0.45 + 120
        x1 = cx + r * 0.7
        s.append(f'<path d="M {x0:.1f} {y0:.1f} A {r} {r} 0 0 1 {x1:.1f} {y0:.1f}" fill="none" stroke="{color}" '
                 f'stroke-width="{width}" stroke-linecap="round" opacity="{op}"/>')
    return "\n".join(s)


# D1 — Echo: a solid bat with two echolocation arcs above (the arcs of the current icon,
# now as the bat's "signal").
b1 = bat_path(cx=512, cy=610, span=0.95, height=1.0)
icon("D1-echo", f'''    <path d="{b1}" fill="url(#p)"/>
{arcs(512, 245, radii=(130, 220), ops=(1, 0.5), width=44)}''')
glyph("D1-echo", f'<path d="{bat_path(cx=512, cy=610, span=1.22, height=1.22)}" fill="#000"/>'
      + arcs(512, 150, radii=(150,), ops=(1,), width=70, color="#000"))

# D2 — Charge level: the bat itself is the gauge, filled to two thirds from below.
b2 = bat_path(cx=512, cy=530, span=0.95, height=1.2)
icon("D2-level", f'''    <path d="{b2}" fill="{PALE}"/>
    <path d="{b2}" fill="url(#p)" clip-path="url(#lvl)"/>''',
     extra_defs='<clipPath id="lvl"><rect x="0" y="470" width="1024" height="554"/></clipPath>')
glyph("D2-level", f'<path d="{bat_path(cx=512, cy=520, span=1.3, height=1.5)}" fill="none" stroke="#000" stroke-width="56" stroke-linejoin="round"/>'
      f'<clipPath id="l"><rect x="0" y="470" width="1024" height="554"/></clipPath>'
      f'<path d="{bat_path(cx=512, cy=520, span=1.3, height=1.5)}" fill="#000" clip-path="url(#l)"/>')

# D3 — Battery bat: the battery of the current icon grows ears and wings.
wing = bat_path(cx=512, cy=560, span=0.95, height=1.0, ears=False)
d3 = f'''    <path d="{wing}" fill="url(#p)" opacity="0.55"/>
    <path d="M 430 432 L 452 362 L 476 432 Z" fill="url(#p)"/>
    <path d="M 548 432 L 572 362 L 594 432 Z" fill="url(#p)"/>
    <rect x="352" y="425" width="320" height="230" rx="56" fill="#FFFFFF" stroke="url(#p)" stroke-width="40"/>
    <rect x="392" y="466" width="72" height="148" rx="20" fill="url(#p)"/>
    <rect x="476" y="466" width="72" height="148" rx="20" fill="url(#p)"/>
    <rect x="560" y="466" width="72" height="148" rx="20" fill="{PALE}"/>'''
icon("D3-battery-bat", d3)
glyph("D3-battery-bat",
      '<mask id="m"><rect width="1024" height="1024" fill="#fff"/><rect x="380" y="460" width="264" height="170" rx="30" fill="#000"/></mask>'
      f'<g mask="url(#m)"><path d="{bat_path(cx=512, cy=560, span=1.3, height=1.15, ears=False)}" fill="#000"/>'
      '<rect x="330" y="410" width="364" height="270" rx="64" fill="#000"/></g>'
      '<path d="M 408 420 L 440 320 L 474 420 Z" fill="#000"/><path d="M 550 420 L 584 320 L 616 420 Z" fill="#000"/>'
      '<rect x="400" y="478" width="96" height="134" rx="22" fill="#000"/><rect x="520" y="478" width="96" height="134" rx="22" fill="#000"/>')

# D4 — Bat in the battery: the current battery outline, the charge drawn as a bat.
d4 = f'''    <rect x="190" y="405" width="600" height="320" rx="80" fill="none" stroke="url(#p)" stroke-width="46"/>
    <rect x="806" y="510" width="46" height="112" rx="18" fill="url(#p)"/>
    <path d="{bat_path(cx=490, cy=575, span=0.72, height=0.72)}" fill="url(#p)"/>
{arcs(490, 225, radii=(110, 190), ops=(1, 0.5), width=38)}'''
icon("D4-bat-in-battery", d4)
glyph("D4-bat-in-battery", '<rect x="90" y="330" width="760" height="420" rx="100" fill="none" stroke="#000" stroke-width="76"/>'
      '<rect x="870" y="470" width="70" height="140" rx="24" fill="#000"/>'
      f'<path d="{bat_path(cx=470, cy=550, span=0.92, height=0.92)}" fill="#000"/>')

# D5 — Minimal: a small bat over a level bar.
b5 = bat_path(cx=512, cy=470, span=1.0, height=1.0)
d5 = f'''    <path d="{b5}" fill="url(#p)"/>
    <rect x="262" y="700" width="500" height="64" rx="32" fill="{PALE}"/>
    <rect x="262" y="700" width="340" height="64" rx="32" fill="url(#p)"/>'''
icon("D5-bat-level-bar", d5)
glyph("D5-bat-level-bar", f'<path d="{bat_path(cx=512, cy=450, span=1.25, height=1.25)}" fill="#000"/>'
      '<rect x="150" y="760" width="724" height="100" rx="50" fill="#000" opacity="0.35"/>'
      '<rect x="150" y="760" width="480" height="100" rx="50" fill="#000"/>')

# Final (chosen 2026-11-20): D1 "Echo", refined — narrower wings (clear of the edge),
# arcs closer to the head, composition centred.
final_bat = bat_path(cx=512, cy=622, span=0.9, height=1.0)
final = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{S}" height="{S}" viewBox="0 0 {S} {S}">
  <!-- battbar app icon: a bat with two echolocation arcs. Generated by gen_bat.py. -->
  <defs>
    <clipPath id="body"><path d="{BODY}"/></clipPath>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{BG_TOP}"/><stop offset="1" stop-color="{BG_BOTTOM}"/></linearGradient>
    <linearGradient id="p" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{G0}"/><stop offset="1" stop-color="{G1}"/></linearGradient>
  </defs>
  <path d="{BODY}" fill="url(#bg)"/>
  <g clip-path="url(#body)">
    <path d="{final_bat}" fill="url(#p)"/>
{arcs(512, 285, radii=(130, 220), ops=(1, 0.5), width=46)}
  </g>
  <path d="{BODY}" fill="none" stroke="{EDGE}" stroke-width="4"/>
</svg>
'''
open("AppIcon.svg", "w").write(final)
menubar = ('<svg xmlns="http://www.w3.org/2000/svg" width="36" height="36" viewBox="0 0 1024 1024">\n'
           '  <!-- Template image for the menu bar (black + alpha; macOS tints it). Same motif as the\n'
           '       app icon: a bat with one echolocation arc. Generated by gen_bat.py. -->\n'
           f'  <path d="{bat_path(cx=512, cy=620, span=1.24, height=1.24)}" fill="#000"/>\n'
           f'  {arcs(512, 150, radii=(150,), ops=(1,), width=72, color="#000")}\n'
           '</svg>\n')
open("MenuBarIcon.svg", "w").write(menubar)
print("ok")
