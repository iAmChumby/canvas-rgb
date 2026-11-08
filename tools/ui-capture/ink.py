# The ink extents of a capture region, column by column and row by row (#240).
#
#   python tools/ui-capture/ink.py .build/ui-capture/displays-react-390x844-dark.png 0 0 390 48
#
# Prints, for the region, the runs of columns that carry ink (a pixel that is
# not the region's background) and, for each run, the ink's first and last
# column. Two captures of the same region can then be compared by eye, number
# by number.
import sys

from PIL import Image


def region(path, x, y, w, h):
    img = Image.open(path).convert("RGB")
    return img.crop((x, y, x + w, y + h))


def runs(path, x, y, w, h, bg, tol=24):
    img = region(path, x, y, w, h)
    px = img.load()
    cols = []
    for cx in range(w):
        ink = 0
        for cy in range(h):
            r, g, b = px[cx, cy]
            if abs(r - bg[0]) > tol or abs(g - bg[1]) > tol or abs(b - bg[2]) > tol:
                ink += 1
        cols.append(ink)
    out = []
    start = None
    for cx, ink in enumerate(cols):
        if ink and start is None:
            start = cx
        elif not ink and start is not None:
            out.append((start + x, cx - 1 + x, cx - start))
            start = None
    if start is not None:
        out.append((start + x, w - 1 + x, w - start))
    return out


def main():
    if len(sys.argv) < 5:
        print(__doc__)
        return 1
    path = sys.argv[1]
    x, y, w, h = (int(v) for v in sys.argv[2:6])
    bg = (18, 20, 23)
    if len(sys.argv) > 6:
        bg = tuple(int(v) for v in sys.argv[6].split(","))
    print(f"{path} region {x},{y},{w},{h} bg {bg}")
    for r in runs(path, x, y, w, h, bg):
        print(f"  ink columns {r[0]}..{r[1]}  (width {r[2]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
