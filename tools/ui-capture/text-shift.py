# Per text box: is GPUI's ink displaced from React's? (#209, #213)
#
#   python tools/ui-capture/text-shift.py react.png gpui.png probe.json 390x844-dark
#
# For every text rect of the probe (probe-page.mjs textRects) it finds the
# integer shift (dx, dy) in -2..2 of the GPUI crop that best matches React's
# crop (greyscale, a pixel differs when it differs by more than 6), and prints
# the rects where a non-zero shift beats the zero shift by 25% or more. A text
# box whose best shift is 0,0 differs only by rasterisation.
import json
import sys

import numpy as np
from PIL import Image

a = np.asarray(Image.open(sys.argv[1]).convert("L")).astype(int)
b = np.asarray(Image.open(sys.argv[2]).convert("L")).astype(int)
data = json.load(open(sys.argv[3]))[sys.argv[4]]
H, W = a.shape
n = zero_best = 0
moved = []
for x, y, w, h in data["textRects"]:
    x0, y0 = int(x) - 1, int(y) - 1
    x1, y1 = min(W - 3, int(x + w) + 2), min(H - 3, int(y + h) + 2)
    if x0 < 2 or y0 < 2 or x1 <= x0 or y1 <= y0:
        continue
    ra = a[y0:y1, x0:x1]
    scores = {}
    for dy in range(-2, 3):
        for dx in range(-2, 3):
            rb = b[y0 + dy:y1 + dy, x0 + dx:x1 + dx]
            scores[(dx, dy)] = int((abs(ra - rb) > 6).sum())
    best = min(scores, key=lambda k: (scores[k], abs(k[0]) + abs(k[1])))
    z = scores[(0, 0)]
    if z == 0:
        continue
    n += 1
    if best == (0, 0) or scores[best] > 0.75 * z:
        zero_best += 1
    else:
        moved.append((z, best, scores[best], (x, y, w, h)))
print(f"text rects with any difference: {n}; best shift is 0,0 (or within 25%): {zero_best}; displaced: {len(moved)}")
for z, best, s, r in sorted(moved, reverse=True):
    print(f"  rect {r[0]:.1f},{r[1]:.1f} {r[2]:.1f}x{r[3]:.1f}: zero-shift {z} px, best shift dx={best[0]:+d} dy={best[1]:+d} {s} px")
