# The difference between two captures, band by band (#240).
#
#   python tools/ui-capture/band-diff.py react.png gpui.png 0,0,390,48 name=strip,x=32,w=80 ...
#
# Each band is "name=x,y,w,h"; the script prints how many pixels differ inside
# it, and how many the band holds.
import sys

from PIL import Image


def main():
    if len(sys.argv) < 4:
        print(__doc__)
        return 1
    a = Image.open(sys.argv[1]).convert("RGB")
    b = Image.open(sys.argv[2]).convert("RGB")
    if a.size != b.size:
        print(f"size mismatch: {a.size} vs {b.size}")
        return 1
    pa = a.load()
    pb = b.load()
    w, h = a.size
    bands = []
    for spec in sys.argv[4:]:
        name, _, rest = spec.partition("=")
        x, y, bw, bh = (int(v) for v in rest.split(","))
        bands.append((name, x, y, bw, bh))
    if not bands:
        bands = [("all", 0, 0, w, h)]
    print(f"{sys.argv[3]}")
    total_diff = 0
    total_px = 0
    for name, x, y, bw, bh in bands:
        diff = 0
        for yy in range(y, min(y + bh, h)):
            for xx in range(x, min(x + bw, w)):
                r1, g1, b1 = pa[xx, yy]
                r2, g2, b2 = pb[xx, yy]
                if abs(r1 - r2) > 6 or abs(g1 - g2) > 6 or abs(b1 - b2) > 6:
                    diff += 1
        n = bw * bh
        total_diff += diff
        total_px += n
        print(f"  {name:22} {diff:6} / {n:6}  ({100.0 * diff / max(n, 1):5.2f}%)")
    if len(bands) > 1:
        print(f"  {'TOTAL':22} {total_diff:6} / {total_px:6}  ({100.0 * total_diff / max(total_px, 1):5.2f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
