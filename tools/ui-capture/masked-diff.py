# Splits a pair's difference into text, icon and the rest (#209, #213).
#
#   python tools/ui-capture/masked-diff.py react.png gpui.png probe.json 390x844-dark [pad]
#
# probe.json is tools/ui-capture/probe-page.mjs output (its textRects and
# iconRects cover the whole document). A pixel differs when any channel differs
# by more than 6 (diff-pairs.ps1's threshold). It is "text" when it lies inside
# a text rect grown by `pad` px (default 2), "icon" when inside an svg rect,
# and "rest" otherwise; the rest is the layout difference. The SVG strips (the
# LED strip, the zone tiles) are icons here, so the rest also lists the
# largest rest clusters.
import json
import sys

import numpy as np
from PIL import Image

a = np.asarray(Image.open(sys.argv[1]).convert("RGB")).astype(int)
b = np.asarray(Image.open(sys.argv[2]).convert("RGB")).astype(int)
data = json.load(open(sys.argv[3]))[sys.argv[4]]
pad = int(sys.argv[5]) if len(sys.argv) > 5 else 2
H, W = a.shape[:2]
diff = (np.abs(a - b).max(axis=2) > 6)
text = np.zeros((H, W), bool)
icon = np.zeros((H, W), bool)
for x, y, w, h in data["textRects"]:
    text[max(0, int(y) - pad):min(H, int(y + h) + pad + 1), max(0, int(x) - pad):min(W, int(x + w) + pad + 1)] = True
for x, y, w, h in data["iconRects"]:
    icon[max(0, int(y) - 1):min(H, int(y + h) + 2), max(0, int(x) - 1):min(W, int(x + w) + 2)] = True
tot = int(diff.sum())
t = int((diff & text).sum())
i = int((diff & ~text & icon).sum())
rest = diff & ~text & ~icon
r = int(rest.sum())
print(f"total={tot}/{W*H} ({100*tot/(W*H):.2f}%)  text={t}  icon-only={i}  rest={r} ({100*r/(W*H):.2f}%)  text-area={int(text.sum())}px")
# rest clusters on a 12 px grid
from collections import defaultdict
cells = defaultdict(int)
ys, xs = np.nonzero(rest)
for y, x in zip(ys, xs):
    cells[(x // 12 * 12, y // 12 * 12)] += 1
for (x, y), n in sorted(cells.items(), key=lambda kv: -kv[1])[:int(sys.argv[6]) if len(sys.argv) > 6 else 12]:
    print(f"  rest cell {x},{y} n={n}")
