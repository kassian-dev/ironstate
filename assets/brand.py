#!/usr/bin/env python3
"""Generate the ironstate brand assets: logo, wordmark, social card, favicon.

One shared mark (a transition arrow into a parenthesized iron tile) drives all
three, so they stay in sync. Run this to regenerate the SVGs, then rasterize
them with resvg (`cargo install resvg`), which honors the viewBox faithfully:

    python3 assets/brand.py            # writes the .svg sources and .png renders

Rendering uses resvg rather than macOS `qlmanage`, which auto-fits visible ink
and renders transparent SVGs off-center.
"""
import math
import os
import shutil
import subprocess

ROOT = os.path.dirname(os.path.abspath(__file__))

SF, SB = "#1f2933", "#3b4a55"      # slate tile fill / border
WH = "#e8edf2"                      # Fe
SG = "#9fb3c4"                      # social-preview gray (subtitle + dark wordmark)
GM = "#64748b"                      # neutral wordmark gray, legible on light or dark
AF, AB = "#f59e0b", "#b45309"      # amber accent fill / darker-amber outline
FONT = "Helvetica, Arial, sans-serif"
DEFS = ('<defs><linearGradient id="tg" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#30404f"/><stop offset="1" stop-color="#141b22"/>'
        '</linearGradient></defs>')


def _path(pts):
    return "M " + " L ".join(f"{x:.2f} {y:.2f}" for x, y in pts) + " Z"


def paren_pts(cx, cy, R, half_deg, wb, we, side, n=44):
    a0, a1 = (180 - half_deg, 180 + half_deg) if side < 0 else (-half_deg, half_deg)
    a0, a1 = math.radians(a0), math.radians(a1)
    outer, inner = [], []
    for i in range(n + 1):
        t = i / n
        a = a0 + (a1 - a0) * t
        hw = we + (wb - we) * math.sin(math.pi * t)  # thick belly, slimmer ends
        ox, oy = math.cos(a), math.sin(a)
        px, py = cx + R * math.cos(a), cy + R * math.sin(a)
        outer.append((px + hw * ox, py + hw * oy))
        inner.append((px - hw * ox, py - hw * oy))
    return outer + inner[::-1]


def arrow_pts(tip, cy, hd, hh, sl, sh):
    return [(tip, cy), (tip - hd, cy - hh), (tip - hd, cy - sh), (tip - hd - sl, cy - sh),
            (tip - hd - sl, cy + sh), (tip - hd, cy + sh), (tip - hd, cy + hh)]


def mark(tw, cx=0.0, cy=0.0):
    """Mark centered at (cx,cy), tile width tw. Returns (svg, (x0,y0,x1,y1))."""
    th = 1.077 * tw; rx = 0.192 * tw; R = 0.84 * tw; half = 46; wb = 0.10 * tw; we = 0.065 * tw
    gap = 0.30 * tw; hd = 0.20 * tw; hh = 0.24 * tw; sl = 0.24 * tw; sh = 0.088 * tw; agap = 0.13 * tw
    fe = 0.625 * tw; nums = 0.154 * tw; sw = max(2.0, 0.029 * tw)
    tl, tt = cx - tw / 2, cy - th / 2
    cx_o = (tl - gap) + R - wb
    cx_c = 2 * cx - cx_o
    open_pts = paren_pts(cx_o, cy, R, half, wb, we, -1)
    close_pts = paren_pts(cx_c, cy, R, half, wb, we, +1)
    apts = arrow_pts((cx_o - R) - wb - agap, cy, hd, hh, sl, sh)
    els = [
        f'<path d="{_path(apts)}" fill="{AF}" stroke="{AB}" stroke-width="{sw:.1f}"/>',
        f'<path d="{_path(open_pts)}" fill="{AF}" stroke="{AB}" stroke-width="{sw:.1f}"/>',
        f'<path d="{_path(close_pts)}" fill="{AF}" stroke="{AB}" stroke-width="{sw:.1f}"/>',
        f'<rect x="{tl:.2f}" y="{tt:.2f}" width="{tw:.2f}" height="{th:.2f}" rx="{rx:.2f}" fill="url(#tg)" stroke="{SB}" stroke-width="{sw:.1f}"/>',
        f'<rect x="{tl+sw:.2f}" y="{tt+sw:.2f}" width="{tw-2*sw:.2f}" height="{th-2*sw:.2f}" rx="{rx-sw:.2f}" fill="none" stroke="#46586a" stroke-width="{max(1.0,sw*0.5):.1f}" stroke-opacity="0.6"/>',
        f'<text x="{tl+0.125*tw:.2f}" y="{tt+0.241*th:.2f}" font-family="{FONT}" font-size="{nums:.1f}" font-weight="700" fill="{AF}">26</text>',
        f'<text x="{cx:.2f}" y="{tt+0.786*th:.2f}" text-anchor="middle" font-family="{FONT}" font-size="{fe:.1f}" font-weight="700" fill="{WH}">Fe</text>',
    ]
    pts = apts + open_pts + close_pts + [(tl, tt), (tl + tw, tt), (tl, tt + th), (tl + tw, tt + th)]
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    return "\n".join("    " + e for e in els), (min(xs) - sw, min(ys) - sw, max(xs) + sw, max(ys) + sw)


def group(svg, dx, dy):
    return f'  <g transform="translate({dx:.2f},{dy:.2f})">\n{svg}\n  </g>'


def write(path, w, h, body, bg=None):
    rect = f'<rect width="{w}" height="{h}" fill="{bg}"/>\n  ' if bg else ""
    out = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" role="img" aria-label="ironstate">\n'
           f'  {DEFS}\n  {rect}{body}\n</svg>\n')
    open(os.path.join(ROOT, path), "w").write(out)


def build():
    # logo: center the true bbox in 256x256
    m, (x0, y0, x1, y1) = mark(90)
    write("logo.svg", 256, 256, group(m, 128 - (x0 + x1) / 2, 128 - (y0 + y1) / 2))

    # wordmark: mark (left) + "ironstate" (760x220). Three text colors so the
    # README <picture> serves a crisp variant per color scheme, with the neutral
    # mid-gray default staying legible on any background — docs.rs embeds the
    # README but keys the swap off the OS scheme, not its own theme switch.
    m, (x0, y0, x1, y1) = mark(100)
    dx, dy = 16 - x0, 110 - (y0 + y1) / 2

    def wm(fill):
        return (group(m, dx, dy) +
                f'\n  <text x="{(x1+dx)+30:.0f}" y="144" font-family="{FONT}" '
                f'font-size="98" font-weight="800" fill="{fill}">ironstate</text>')

    write("wordmark.svg", 760, 220, wm(GM))         # neutral default / fallback
    write("wordmark-light.svg", 760, 220, wm(SF))   # dark slate, light backgrounds
    write("wordmark-dark.svg", 760, 220, wm(SG))    # social gray, dark backgrounds

    # social card: mark (left) + text block (right), dark bg
    m, (x0, y0, x1, y1) = mark(176)
    dx, dy = 70 - x0, 320 - (y0 + y1) / 2
    tx = 560
    write("social-preview.svg", 1280, 640, group(m, dx, dy) +
          f'\n  <text x="{tx}" y="262" font-family="{FONT}" font-size="116" font-weight="800" fill="{WH}">ironstate</text>'
          f'\n  <text x="{tx+2}" y="332" font-family="{FONT}" font-size="40" font-weight="400" fill="{SG}">Verified state machines</text>'
          f'\n  <text x="{tx+2}" y="384" font-family="{FONT}" font-size="40" font-weight="400" fill="{SG}">for humans and AI agents</text>'
          f'\n  <text x="{tx+4}" y="452" font-family="{FONT}" font-size="30" font-weight="600" fill="{AF}">decide · evolve · replay · verify</text>',
          bg="#11181f")


def render():
    resvg = shutil.which("resvg")
    if not resvg:
        print("resvg not found (cargo install resvg) — wrote SVGs only")
        return
    jobs = [("logo.svg", "logo.png", 512), ("logo.svg", "favicon-32.png", 32),
            ("wordmark.svg", "wordmark.png", 760),
            ("wordmark-light.svg", "wordmark-light.png", 760),
            ("wordmark-dark.svg", "wordmark-dark.png", 760),
            ("social-preview.svg", "social-preview.png", 1280)]
    for src, out, w in jobs:
        subprocess.run([resvg, "--width", str(w), os.path.join(ROOT, src), os.path.join(ROOT, out)], check=True)
    print("wrote SVGs + PNGs")


if __name__ == "__main__":
    build()
    render()
