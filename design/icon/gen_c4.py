#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Concept C4 (white + periwinkle) with improved legibility, on the macOS icon grid."""
from gen_concepts import S, BODY

def c4(name, bg_top, bg_bottom, edge, g0, g1, scale=1.0, outline_w=40, arc_w=34, arcs=3,
       dim="opacity", arc_ops=(1, 0.7, 0.45), show_arcs=True, dy=0):
    cx, cy = 512, 545 + dy
    def tx(x, y):  # scale around the glyph centre
        return cx + (x - 512) * scale, cy + (y - 545) * scale
    g = []
    x, y = tx(210, 420)
    g.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{560*scale:.1f}" height="{300*scale:.1f}" rx="{70*scale:.1f}" fill="none" stroke="url(#p)" stroke-width="{outline_w*scale:.1f}"/>')
    x, y = tx(770, 515)
    g.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{44*scale:.1f}" height="{110*scale:.1f}" rx="{16*scale:.1f}" fill="url(#p)"/>')
    for i in range(3):
        x, y = tx(262 + i * 162, 472)
        if i < 2:
            fill, op = "url(#p)", 1
        else:
            fill, op = ("url(#p)", 0.3) if dim == "opacity" else ("#D9DEFB", 1)
        g.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{132*scale:.1f}" height="{196*scale:.1f}" rx="{30*scale:.1f}" fill="{fill}" opacity="{op}"/>')
    if show_arcs:
        radii = (120, 200, 280)[:arcs] if arcs == 3 else (140, 240)
        for r, op in zip(radii, arc_ops):
            ax0, ay0 = tx(512 - r * 0.7, 370 - r * 0.45)
            ax1, ay1 = tx(512 + r * 0.7, 370 - r * 0.45)
            g.append(f'<path d="M {ax0:.1f} {ay0:.1f} A {r*scale:.1f} {r*scale:.1f} 0 0 1 {ax1:.1f} {ay1:.1f}" fill="none" stroke="url(#p)" stroke-width="{arc_w*scale:.1f}" stroke-linecap="round" opacity="{op}"/>')
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{S}" height="{S}" viewBox="0 0 {S} {S}">
  <defs>
    <clipPath id="body"><path d="{BODY}"/></clipPath>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{bg_top}"/><stop offset="1" stop-color="{bg_bottom}"/></linearGradient>
    <linearGradient id="p" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{g0}"/><stop offset="1" stop-color="{g1}"/></linearGradient>
  </defs>
  <path d="{BODY}" fill="url(#bg)"/>
  <g clip-path="url(#body)">
    {chr(10).join("    " + s for s in g).strip()}
  </g>
  <path d="{BODY}" fill="none" stroke="{edge}" stroke-width="4"/>
</svg>
'''
    open(f"concepts/{name}.svg", "w").write(svg)

# C4 original for reference
c4("C4-0-original", "#FFFFFF", "#F1F2F7", "#E1E3EC", "#9DB4FF", "#B79CF2")
# a: deeper gradient, thicker strokes, slightly larger glyph
c4("C4-a-deeper", "#FFFFFF", "#F0F1F8", "#DADDEB", "#7A93F7", "#9673E6", scale=1.06, outline_w=46, arc_w=40)
# b: + pale lavender background, two bigger arcs
c4("C4-b-tinted-2arcs", "#FBFBFF", "#E9ECFA", "#D3D8EF", "#6F89F4", "#8E68E3", scale=1.1, outline_w=48, arc_w=44,
   arcs=2, arc_ops=(1, 0.6))
# c: + solid light empty segment (no transparency), strongest contrast
c4("C4-c-solid", "#FBFBFF", "#E9ECFA", "#D3D8EF", "#6580F2", "#8A60E0", scale=1.12, outline_w=50, arc_w=46,
   arcs=2, arc_ops=(1, 0.65), dim="solid")
# small-size rendition (16/32 px): battery only, bigger, no arcs
c4("C4-c-small", "#FBFBFF", "#E9ECFA", "#C9CFEA", "#5D78F0", "#8659DE", scale=1.32, outline_w=56, dim="solid",
   show_arcs=False, dy=-35)
print("ok")
