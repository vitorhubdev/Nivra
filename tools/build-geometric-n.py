#!/usr/bin/env python3
"""Generate the clean geometric N for ETAPA A (no IoU gate, geometric gate only).
Params from the owner's brief: w=556, h=600, stem=152, diagonal wide (a0=-111, s=567, dw=234), r=10.
"""
from pathlib import Path
from shapely.geometry import box, Polygon
from shapely.ops import unary_union

ROOT = Path("C:/Users/jzv/Desktop/Nivra-visual")
BRAND = ROOT / "assets" / "brand"

W,H,SW = 556,600,152
A0,S,DW = -111,567,234
R = 10

A = box(0,0,SW,H)
B = box(W-SW,0,W,H)
C = Polygon([(A0,0),(A0+DW,0),(A0+S+DW,H),(A0+S,H)])
union = unary_union([A,B,C]).intersection(box(0,0,W,H))
rounded = union.buffer(R, join_style='round', resolution=8).buffer(-R, join_style='round', resolution=8)
coords = list(rounded.exterior.coords)
print(f"union area {union.area:.1f} rounded area {rounded.area:.1f} pts {len(coords)} bounds {rounded.bounds}")

def fmt(v):
    s = f"{v:.2f}".rstrip('0').rstrip('.')
    return '0' if s in ('','-0') else s

def path_d(pts):
    # pts: list of (x,y), last == first for closed; emit M + L... + Z
    d = [f"M {fmt(pts[0][0])} {fmt(pts[0][1])}"]
    for x,y in pts[1:]:
        # skip duplicate closing point, Z will close
        d.append(f"L {fmt(x)} {fmt(y)}")
    # remove last if equals first
    if abs(pts[-1][0]-pts[0][0])<1e-9 and abs(pts[-1][1]-pts[0][1])<1e-9:
        d.pop()
    d.append("Z")
    return " ".join(d)

# base local path (0..W,0..H)
base_pts = coords
base_d = path_d(base_pts)

def shifted_d(dx, dy, scale=1.0):
    pts = [(x*scale+dx, y*scale+dy) for x,y in base_pts]
    return path_d(pts)

# mark: square 604, N 556x600 centred -> tx=24, ty=2
MARK_SIDE = 604.0
mark_tx = (MARK_SIDE - W)/2
mark_ty = (MARK_SIDE - H)/2
print(f"mark side {MARK_SIDE} tx {mark_tx} ty {mark_ty}")
mark_d = shifted_d(mark_tx, mark_ty)

# icon: 1024 canvas, N centred -> tx=234, ty=212
icon_tx = (1024 - W)/2
icon_ty = (1024 - H)/2
print(f"icon tx {icon_tx} ty {icon_ty}")
icon_d = shifted_d(icon_tx, icon_ty)

# symbolic: fit inside nivra bbox 187.56..833.39 x 173.37..826.30 (w645.83 h652.93), preserve aspect
sx0,sy0,sx1,sy1 = 187.56,173.37,833.39,826.30
sw, sh = sx1-sx0, sy1-sy0
scale = min(sw/W, sh/H)
print(f"nivra bbox w {sw:.2f} h {sh:.2f} scale {scale:.5f}")
nw, nh = W*scale, H*scale
sym_tx = (1024 - nw)/2
sym_ty = (1024 - nh)/2
print(f"symbolic nw {nw:.2f} nh {nh:.2f} tx {sym_tx:.2f} ty {sym_ty:.2f} margins L {sym_tx:.1f} R {1024-nw-sym_tx:.1f} T {sym_ty:.1f} B {1024-nh-sym_ty:.1f}")
sym_d = shifted_d(sym_tx, sym_ty, scale)

# write a json for reference
import json
(ROOT / "tools" / "geometric-n.json").write_text(json.dumps({
    "W":W,"H":H,"SW":SW,"A0":A0,"S":S,"DW":DW,"R":R,
    "mark_side":MARK_SIDE,"mark_tx":mark_tx,"mark_ty":mark_ty,
    "icon_tx":icon_tx,"icon_ty":icon_ty,
    "sym_scale":scale,"sym_tx":sym_tx,"sym_ty":sym_ty,
    "pts":len(coords),
}, indent=2), encoding="utf-8")
print("wrote tools/geometric-n.json")

# --- build SVGs, reusing plate from current nivra.svg ---
import xml.etree.ElementTree as ET
cur = (BRAND / "nivra.svg").read_text(encoding="utf-8")
# extract plate d: first <path ... d="..."> with fill url(#plate)
import re
m = re.search(r'<path d="([^"]+)" fill="url\(#plate\)"', cur)
plate_d = m.group(1)
print(f"plate d len {len(plate_d)}")

# nivra.svg
svg_head = '''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>Nivra</title>
  <desc>Nivra application mark: a glass N on a violet-to-blue plate.</desc>
  <defs>
    <linearGradient id="plate" x1="0" y1="0" x2="1" y2="1"><stop offset="0.0" stop-color="#F0C4FE"/><stop offset="0.18" stop-color="#C77BFA"/><stop offset="0.38" stop-color="#8B44F4"/><stop offset="0.56" stop-color="#3E27ED"/><stop offset="0.74" stop-color="#0A6BFD"/><stop offset="0.88" stop-color="#2FB6FD"/><stop offset="1.0" stop-color="#9BDCFE"/></linearGradient>
    <radialGradient id="core" cx="0.5" cy="0.5" r="0.66"><stop offset="0" stop-color="#0E0750" stop-opacity="0.60"/><stop offset="0.5" stop-color="#1A0C74" stop-opacity="0.32"/><stop offset="1" stop-color="#1A0C74" stop-opacity="0"/></radialGradient>
    <linearGradient id="glass" x1="0.1" y1="0" x2="0.7" y2="1"><stop offset="0" stop-color="#FFFFFF" stop-opacity="0.94"/><stop offset="0.45" stop-color="#EADCFF" stop-opacity="0.84"/><stop offset="1" stop-color="#AE8CFF" stop-opacity="0.74"/></linearGradient>
  </defs>
'''
plate_paths = f'''  <path d="{plate_d}" fill="url(#plate)"/>
  <path d="{plate_d}" fill="url(#core)"/>
  <path d="{plate_d}" fill="none" stroke="#F6EAFF" stroke-opacity="0.42" stroke-width="7"/>
'''
n_path_glass = f'''  <path d="{icon_d}" fill="url(#glass)"/>
</svg>
'''
(BRAND / "nivra.svg").write_text(svg_head + plate_paths + n_path_glass, encoding="utf-8", newline="\n")
print("wrote nivra.svg")

# nivra-flat.svg
flat = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>Nivra</title>
  <desc>Nivra application mark, flat single-colour variant for small sizes.</desc>
  <path d="{plate_d}" fill="#5B2BE0"/>
  <path d="{icon_d}" fill="#F3EAFF"/>
</svg>
'''
(BRAND / "nivra-flat.svg").write_text(flat, encoding="utf-8", newline="\n")
print("wrote nivra-flat.svg")

# nivra-mark.svg
mark_svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{MARK_SIDE:.2f}" height="{MARK_SIDE:.2f}" viewBox="0 0 {MARK_SIDE:.2f} {MARK_SIDE:.2f}" fill="none">
  <title>Nivra mark</title>
  <desc>Nivra N monogram, solid white silhouette, square view box with the mark centred.</desc>
  <path fill="#fff" d="{mark_d}"/>
</svg>
'''
(BRAND / "nivra-mark.svg").write_text(mark_svg, encoding="utf-8", newline="\n")
print("wrote nivra-mark.svg")

# symbolic (packaging) with nivra-like margin
sym_svg = f'''<?xml version='1.0' encoding='UTF-8'?>
<svg xmlns='http://www.w3.org/2000/svg' width='1024' height='1024' viewBox='0 0 1024 1024' fill='none'>
  <title>Nivra symbolic icon</title>
  <desc>Nivra N monogram, monochrome, centred on the 1024 canvas with the historic symbolic margin.</desc>
<path fill="#000000" d="{sym_d}"/>
</svg>
'''
(ROOT / "packaging" / "linux" / "hicolor" / "scalable" / "apps" / "nivra-symbolic.svg").write_text(sym_svg, encoding="utf-8", newline="\n")
print("wrote nivra-symbolic.svg")

# scalable copy
import shutil
shutil.copyfile(BRAND / "nivra.svg", ROOT / "packaging" / "linux" / "hicolor" / "scalable" / "apps" / "nivra.svg")
print("copied scalable nivra.svg")
