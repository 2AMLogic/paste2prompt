"""Adds a U+FFFD REPLACEMENT CHARACTER glyph to the Inter Regular subset.

Inter has no U+FFFD glyph, and the subset's `.notdef` has no outline, so
without this a character the subset can't map would render as nothing. The
renderer replaces such characters with U+FFFD (see `src/render/text.rs`),
and this script gives U+FFFD a visible shape: the usual black diamond with
Inter's own question mark cut out of it. Inter is OFL-1.1 with no Reserved
Font Name, so a modified subset may keep the name (see `OFL.txt`).

Run after `pyftsubset`, from `crates/core` (needs fontTools):

    python3 fonts/add_replacement_glyph.py fonts/Inter-Regular-subset.ttf

Idempotent: a font that already maps U+FFFD is left unchanged.
"""

import sys

from fontTools.pens.reverseContourPen import ReverseContourPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

NAME = "uniFFFD"


def main(path: str) -> None:
    font = TTFont(path)
    cmap = font.getBestCmap()
    if 0xFFFD in cmap:
        print(f"{path}: U+FFFD already mapped to {cmap[0xFFFD]}")
        return

    glyf = font["glyf"]
    glyphs = font.getGlyphSet()
    cap = font["OS/2"].sCapHeight
    question = cmap[ord("?")]

    # A diamond from a little below the baseline to a little above cap
    # height, centered on the cap height's middle.
    half_h = cap * 0.6
    half_w = half_h * 0.88
    side = round(cap * 0.05)
    cx, cy = side + half_w, cap / 2

    pen = TTGlyphPen(glyphs)
    # Clockwise: TrueType's direction for a filled outer contour.
    pen.moveTo((cx, cy + half_h))
    pen.lineTo((cx + half_w, cy))
    pen.lineTo((cx, cy - half_h))
    pen.lineTo((cx - half_w, cy))
    pen.closePath()

    # The question mark, scaled into the diamond and drawn with reversed
    # contours, so the nonzero fill cuts it out.
    q = glyf[question]
    q.recalcBounds(glyf)
    scale = 0.62
    qcx, qcy = (q.xMin + q.xMax) / 2, (q.yMin + q.yMax) / 2
    transform = (scale, 0, 0, scale, cx - qcx * scale, cy - qcy * scale)
    glyphs[question].draw(TransformPen(ReverseContourPen(pen), transform))

    glyph = pen.glyph()
    order = font.getGlyphOrder()
    order.append(NAME)
    font.setGlyphOrder(order)
    glyf[NAME] = glyph
    glyph.recalcBounds(glyf)
    font["hmtx"][NAME] = (round(2 * (side + half_w)), glyph.xMin)
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[0xFFFD] = NAME
    font.save(path)
    print(f"{path}: added {NAME} for U+FFFD")


if __name__ == "__main__":
    main(sys.argv[1])
