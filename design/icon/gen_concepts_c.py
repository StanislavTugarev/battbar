#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Concept C ("signal battery") in soft, mostly white variants, on the macOS icon grid."""
import math
from gen_concepts import S, BODY

def variant(name, bg_top, bg_bottom, edge, outline, seg_fill, seg_dim_opacity, arcs, arc_opacities=(1, 0.7, 0.45), extra_defs="", seg_colors=None):
    defs = f'''<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{bg_top}"/><stop offset="1" stop-color="{bg_bottom}"/></linearGradient>
    {extra_defs}'''
    g = []
    g.append(f'<rect x="210" y="420" width="560" height="300" rx="70" fill="none" stroke="{outline}" stroke-width="40"/>')
    g.append(f'<rect x="770" y="515" width="44" height="110" rx="16" fill="{outline}"/>')
    for i in range(3):
        op = [1, 1, seg_dim_opacity][i]
        fill = seg_colors[i] if seg_colors else seg_fill
        g.append(f'<rect x="{262 + i * 162}" y="472" width="132" height="196" rx="30" fill="{fill}" opacity="{op}"/>')
    for (r, op) in zip((120, 200, 280), arc_opacities):
        g.append(f'<path d="M {512 - r * 0.7:.1f} {370 - r * 0.45:.1f} A {r} {r} 0 0 1 {512 + r * 0.7:.1f} {370 - r * 0.45:.1f}" '
                 f'fill="none" stroke="{arcs}" stroke-width="34" stroke-linecap="round" opacity="{op}"/>')
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{S}" height="{S}" viewBox="0 0 {S} {S}">
  <defs>
    <clipPath id="body"><path d="{BODY}"/></clipPath>
    {defs}
  </defs>
  <path d="{BODY}" fill="url(#bg)"/>
  <g clip-path="url(#body)">
    {chr(10).join("    " + x for x in g).strip()}
  </g>
  <path d="{BODY}" fill="none" stroke="{edge}" stroke-width="4"/>
</svg>
'''
    with open(f"concepts/{name}.svg", "w") as f:
        f.write(svg)

# C1 — white, soft peach glyph (closest to the original C, much lighter).
variant("C1-white-peach", "#FFFFFF", "#F3F1EF", "#E4E1DE",
        outline="url(#peach)", seg_fill="url(#peach)", seg_dim_opacity=0.3, arcs="url(#peach)",
        extra_defs='''<linearGradient id="peach" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#FFB38A"/><stop offset="1" stop-color="#F28CA8"/></linearGradient>''')
# C2 — white, graphite outline, soft mint segments, light grey signal.
variant("C2-white-mint", "#FFFFFF", "#F2F3F5", "#E2E4E8",
        outline="#4A4F5A", seg_fill="url(#mint)", seg_dim_opacity=0.3, arcs="#A9AFBA",
        extra_defs='''<linearGradient id="mint" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#8EE3B5"/><stop offset="1" stop-color="#4CC48A"/></linearGradient>''')
# C3 — pale blush background, white glyph with a soft shadow.
variant("C3-blush-white", "#FFE9DF", "#F9D3DC", "#EEC9CF",
        outline="#FFFFFF", seg_fill="#FFFFFF", seg_dim_opacity=0.45, arcs="#FFFFFF", arc_opacities=(1, 0.8, 0.6))
# C4 — white, soft periwinkle glyph.
variant("C4-white-periwinkle", "#FFFFFF", "#F1F2F7", "#E1E3EC",
        outline="url(#peri)", seg_fill="url(#peri)", seg_dim_opacity=0.3, arcs="url(#peri)",
        extra_defs='''<linearGradient id="peri" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#9DB4FF"/><stop offset="1" stop-color="#B79CF2"/></linearGradient>''')
# C5 — white, graphite glyph, a single soft coral full segment.
variant("C5-white-graphite-coral", "#FFFFFF", "#F2F2F3", "#E3E3E6",
        outline="#3F434C", seg_fill="#3F434C", seg_dim_opacity=0.18, arcs="#3F434C", arc_opacities=(0.9, 0.55, 0.3),
        seg_colors=["#F49A8A", "#F49A8A", "#3F434C"])
print("ok")
