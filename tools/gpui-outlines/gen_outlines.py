"""Emit the glyph outlines of the characters the GPUI editor draws as outlines.

JetBrains Mono 400 (the editor's `--fm` face), from the file the editor
embeds. Every contour is flattened to a polygon: a quadratic becomes 12
segments, which at 14 px (1 font unit = 0.014 px) is under 0.01 px of error.

Usage:
    python tools/gpui-outlines/gen_outlines.py
"""
import os
import sys

from fontTools.ttLib import TTFont
from fontTools.pens.recordingPen import RecordingPen

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
FONT = os.path.join(ROOT, 'app', 'gpui', 'assets', 'fonts', 'jetbrains-mono-400.ttf')
OUT = os.path.join(ROOT, 'app', 'gpui', 'src', 'components', 'outlines.rs')

CHARS = 'Full speed'
STEPS = 12


def flatten(pen_points):
    """A recording pen's points -> a list of contours of points."""
    contours = []
    current = []
    for op, args in pen_points:
        if op == 'moveTo':
            if current:
                contours.append(current)
            current = [args[0]]
        elif op == 'lineTo':
            current.append(args[0])
        elif op == 'qCurveTo':
            # qCurveTo can carry several off-curve points; the last is on-curve.
            pts = list(args)
            if len(pts) == 1:
                # A closed contour: the last point is the start.
                current.append(pts[0])
                contours.append(current)
                current = []
                continue
            on = pts[-1]
            ctrl = pts[-2]
            for i, off in enumerate(pts[:-2]):
                nxt = pts[i + 1] if i + 1 < len(pts) - 2 else on
                current.extend(quad(ctrl, off, nxt))
            ctrl = on
            current.append(on)
        elif op == 'curveTo':
            # Cubic: approximate with 12 straight segments.
            p0 = current[-1] if current else (0, 0)
            c1, c2, p3 = args
            for i in range(1, STEPS + 1):
                t = i / STEPS
                mt = 1 - t
                x = (mt ** 3) * p0[0] + 3 * (mt ** 2) * t * c1[0] + 3 * mt * (t ** 2) * c2[0] + (t ** 3) * p3[0]
                y = (mt ** 3) * p0[1] + 3 * (mt ** 2) * t * c1[1] + 3 * mt * (t ** 2) * c2[1] + (t ** 3) * p3[1]
                current.append((x, y))
        elif op == 'closePath':
            if current:
                contours.append(current)
            current = []
        elif op == 'endPath':
            if current:
                contours.append(current)
            current = []
    if current:
        contours.append(current)
    return contours


def quad(p0, c, p1):
    pts = []
    for i in range(1, STEPS + 1):
        t = i / STEPS
        mt = 1 - t
        x = (mt ** 2) * p0[0] + 2 * mt * t * c[0] + (t ** 2) * p1[0]
        y = (mt ** 2) * p0[1] + 2 * mt * t * c[1] + (t ** 2) * p1[1]
        pts.append((x, y))
    return pts


def main():
    font = TTFont(FONT)
    upem = font['head'].unitsPerEm
    cmap = font.getBestCmap()
    hmtx = font['hmtx']
    glyph_set = font.getGlyphSet()

    lines = []
    lines.append('//! The outlines of the characters the GPUI editor draws as paths.')
    lines.append('//!')
    lines.append('//! gpui 0.2.2 cannot turn text, so the one caption that stands on its')
    lines.append('//! side (the curve chart\'s "Full speed") is drawn from these outlines')
    lines.append('//! instead. They come from the editor\'s own figures face')
    lines.append('//! (`assets/fonts/jetbrains-mono-400.ttf`, the `--fm` family), so the')
    lines.append('//! shapes are the ones the React editor\'s SVG draws.')
    lines.append('//!')
    lines.append('//! The data is generated: `tools/gpui-outlines/gen_outlines.py`.')
    lines.append('')
    lines.append('/// The face\'s units per em.')
    lines.append(f'pub const UNITS_PER_EM: f32 = {upem}.0;')
    lines.append('/// The advance of one character (the face is monospaced).')
    advance = hmtx[cmap[ord("F")]][0]
    lines.append(f'pub const ADVANCE: f32 = {advance}.0;')
    lines.append('')
    lines.append('/// One character: its contours, in font units.')
    lines.append('pub struct Outline {')
    lines.append('    /// The contours of the glyph; each is a closed run of points.')
    lines.append('    pub contours: &\'static [&\'static [(f32, f32)]],')
    lines.append('}')
    lines.append('')
    lines.append('/// The outlines, by character.')
    lines.append('pub const OUTLINES: &[(char, Outline)] = &[')
    for ch in sorted(set(CHARS)):
        if ch == ' ':
            continue
        name = cmap[ord(ch)]
        pen = RecordingPen()
        glyph_set[name].draw(pen)
        contours = flatten(pen.value)
        lines.append(f'    (\'{ch}\', Outline {{')
        lines.append('        contours: &[')
        for contour in contours:
            points = ', '.join(f'({x:.1f}, {y:.1f})' for x, y in contour)
            lines.append(f'            &[{points}],')
        lines.append('        ],')
        lines.append('    }),')
    lines.append('];')
    lines.append('')
    with open(OUT, 'w', encoding='utf-8', newline='\n') as fh:
        fh.write('\n'.join(lines))
    print(f'wrote {OUT} ({len(lines)} lines)')


if __name__ == '__main__':
    sys.exit(main())
