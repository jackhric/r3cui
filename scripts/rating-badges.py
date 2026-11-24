#!/usr/bin/env python3
"""Draw the certification badges the poster wall shows (PG-13, TV-MA, ...) as SVGs.

The lettering is converted to outlines, so the remote needs no font to show them, and every
mark is white on transparent: the UI tints them. MPA ratings are an outlined box; TV ratings a
filled "TV" block joined to an outlined rating, after the TV Parental Guidelines icons.

    python3 scripts/rating-badges.py /usr/share/fonts/TTF/RobotoCondensed-Bold.ttf crates/ui/ui/ratings
"""

import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

MPA = ["G", "PG", "PG-13", "R", "NC-17", "NR"]
TV = ["TV-Y", "TV-Y7", "TV-G", "TV-PG", "TV-14", "TV-MA"]

H = 20.0  # badge height, in SVG units
CAP = 11.0  # height of the capitals
PAD = 4.5  # space either side of the lettering
STROKE = 1.6
RADIUS = 2.5


def text_path(font, text, x, baseline):
    """Outline of `text` with its baseline at `baseline`, starting at `x`; returns (path, width)."""
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    scale = CAP / font["OS/2"].sCapHeight
    pen = SVGPathPen(glyphs)
    cursor = 0.0
    for ch in text:
        name = cmap[ord(ch)]
        glyph = glyphs[name]
        glyph.draw(TransformPen(pen, (scale, 0, 0, -scale, x + cursor * scale, baseline)))
        cursor += glyph.width
    return pen.getCommands(), cursor * scale


def rounded(x, y, w, h, r):
    return (
        f"M{x + r:.2f} {y:.2f}H{x + w - r:.2f}A{r} {r} 0 0 1 {x + w:.2f} {y + r:.2f}"
        f"V{y + h - r:.2f}A{r} {r} 0 0 1 {x + w - r:.2f} {y + h:.2f}H{x + r:.2f}"
        f"A{r} {r} 0 0 1 {x:.2f} {y + h - r:.2f}V{y + r:.2f}A{r} {r} 0 0 1 {x + r:.2f} {y:.2f}Z"
    )


def ring(x, y, w, h, r):
    s = STROKE
    return rounded(x, y, w, h, r) + rounded(x + s, y + s, w - 2 * s, h - 2 * s, max(0.5, r - s))


def baseline():
    return (H + CAP) / 2


def mpa(font, label):
    _, tw = text_path(font, label, 0, 0)
    w = tw + 2 * PAD + 2 * STROKE
    letters, _ = text_path(font, label, PAD + STROKE, baseline())
    body = f'<path fill="#fff" fill-rule="evenodd" d="{ring(0, 0, w, H, RADIUS)}"/><path fill="#fff" d="{letters}"/>'
    return w, body


def tv(font, label):
    rating = label.removeprefix("TV-")
    _, tvw = text_path(font, "TV", 0, 0)
    _, rw = text_path(font, rating, 0, 0)
    left = tvw + 2 * (PAD - 1)
    w = left + rw + 2 * PAD + STROKE
    # The filled block with "TV" knocked out of it, then the outlined box holding the rating.
    tv_letters, _ = text_path(font, "TV", PAD - 1, baseline())
    block = (
        f"M{RADIUS} 0H{left}V{H}H{RADIUS}A{RADIUS} {RADIUS} 0 0 1 0 {H - RADIUS}V{RADIUS}"
        f"A{RADIUS} {RADIUS} 0 0 1 {RADIUS} 0Z"
    )
    s = STROKE
    box = (
        f"M{left} 0H{w - RADIUS}A{RADIUS} {RADIUS} 0 0 1 {w} {RADIUS}V{H - RADIUS}"
        f"A{RADIUS} {RADIUS} 0 0 1 {w - RADIUS} {H}H{left}V{H - s}H{w - RADIUS}"
        f"A{RADIUS - s} {RADIUS - s} 0 0 0 {w - s} {H - RADIUS}V{RADIUS}"
        f"A{RADIUS - s} {RADIUS - s} 0 0 0 {w - RADIUS} {s}H{left}Z"
    )
    r_letters, _ = text_path(font, rating, left + PAD, baseline())
    # A mask, not an even-odd cut: the font's letters are built from overlapping shapes, which an
    # even-odd fill would punch back in.
    body = (
        f'<mask id="tv"><rect width="{w:.2f}" height="{H}" fill="#fff"/><path fill="#000" d="{tv_letters}"/></mask>'
        f'<path fill="#fff" mask="url(#tv)" d="{block}"/>'
        f'<path fill="#fff" d="{box}"/><path fill="#fff" d="{r_letters}"/>'
    )
    return w, body


def main():
    font = TTFont(sys.argv[1])
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    for label in MPA + TV:
        w, body = (tv if label.startswith("TV-") else mpa)(font, label)
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.2f} {H}" width="{w:.2f}" height="{H}">{body}</svg>\n'
        (out / f"{label.lower()}.svg").write_text(svg)
        print(f"{label}: {w:.1f} x {H}")


if __name__ == "__main__":
    main()
