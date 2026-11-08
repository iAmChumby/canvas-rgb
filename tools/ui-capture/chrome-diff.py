# The shell's own chrome in a pair: the strip and the dock (phone) or the rail (#240, #201).
#
#   python tools/ui-capture/chrome-diff.py react.png gpui.png probe.json 390x844-dark [--boxes]
#
# probe.json is tools/ui-capture/probe-page.mjs output. Regions: at 390 wide the
# strip (0,0,W,48) and the dock (0,H-65,W,65); wider, the strip (200,0,W-200,55)
# and the rail (0,0,200,H). For each region it prints the pixels that differ by
# more than 6 (diff-pairs.ps1's threshold), split into pixels inside React's
# text boxes (2 px pad), inside its svg boxes (1 px pad) and the rest, plus the
# count at thresholds 24 / 48 / 96. With --boxes it also lists every text or
# icon box whose ink bounding box (pixels more than 24 from the box's own
# background) differs between the two images, so displaced ink is told apart
# from rasterisation.
import json
import sys

import numpy as np
from PIL import Image

args = [a for a in sys.argv[1:] if not a.startswith("--")]
boxes = "--boxes" in sys.argv
a = np.asarray(Image.open(args[0]).convert("RGB")).astype(int)
b = np.asarray(Image.open(args[1]).convert("RGB")).astype(int)
data = json.load(open(args[2]))[args[3]]
H, W = a.shape[:2]
if W <= 720:
    regions = {"strip": (0, 0, W, 48), "dock": (0, H - 65, W, 65)}
else:
    regions = {"strip": (200, 0, W - 200, 55), "rail": (0, 0, 200, H)}
d = np.abs(a - b).max(axis=2)
text = np.zeros((H, W), bool)
icon = np.zeros((H, W), bool)
for x, y, w, h in data["textRects"]:
    text[max(0, int(y) - 2):min(H, int(y + h) + 3), max(0, int(x) - 2):min(W, int(x + w) + 3)] = True
for x, y, w, h in data["iconRects"]:
    icon[max(0, int(y) - 1):min(H, int(y + h) + 2), max(0, int(x) - 1):min(W, int(x + w) + 2)] = True


def ink_box(img, x0, y0, x1, y1):
    crop = img[y0:y1, x0:x1]
    bg = np.median(crop.reshape(-1, 3), axis=0)
    m = np.abs(crop - bg).max(axis=2) > 24
    if not m.any():
        return None
    ys, xs = np.nonzero(m)
    return (x0 + xs.min(), y0 + ys.min(), x0 + xs.max() + 1, y0 + ys.max() + 1)


for name, (rx, ry, rw, rh) in regions.items():
    sl = (slice(ry, ry + rh), slice(rx, rx + rw))
    dd = d[sl] > 6
    t = int((dd & text[sl]).sum())
    i = int((dd & ~text[sl] & icon[sl]).sum())
    r = int((dd & ~text[sl] & ~icon[sl]).sum())
    print(
        f"{name} ({rx},{ry},{rw},{rh}): differing={int(dd.sum())}  text={t} icon-only={i} rest={r}  "
        f">24={int((d[sl] > 24).sum())} >48={int((d[sl] > 48).sum())} >96={int((d[sl] > 96).sum())}"
    )
    if boxes:
        rects = [("text", q) for q in data["textRects"]] + [("icon", q) for q in data["iconRects"]]
        for kind, (x, y, w, h) in rects:
            if not (rx <= x < rx + rw and ry <= y < ry + rh):
                continue
            x0, y0 = max(0, int(x) - 3), max(0, int(y) - 3)
            x1, y1 = min(W, int(x + w) + 4), min(H, int(y + h) + 4)
            ba, bb = ink_box(a, x0, y0, x1, y1), ink_box(b, x0, y0, x1, y1)
            if ba == bb:
                continue
            print(f"  {kind} {x:.1f},{y:.1f} {w:.1f}x{h:.1f}: react ink {ba} gpui ink {bb}")
