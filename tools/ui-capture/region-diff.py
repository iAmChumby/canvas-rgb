# The difference between two captures over a rectangle, in diff-pairs.ps1's
# terms (a channel differs by more than 6). (#240)
#
#   python tools/ui-capture/region-diff.py react.png gpui.png 0 0 390 48
#
# Prints "region x,y,w,h differing=N/T (P%)", the same line diff-pairs.ps1
# prints, for any two files (so a grayscale-A/B pair can be measured too).
import sys

from PIL import Image

THRESHOLD = 6


def main():
    if len(sys.argv) < 6:
        print(__doc__)
        return 1
    a = Image.open(sys.argv[1]).convert("RGB")
    b = Image.open(sys.argv[2]).convert("RGB")
    if a.size != b.size:
        print(f"size mismatch: {a.size} vs {b.size}")
        return 1
    x, y, w, h = (int(v) for v in sys.argv[3:7])
    pa, pb = a.load(), b.load()
    W, H = a.size
    diff = 0
    total = 0
    for yy in range(max(0, y), min(y + h, H)):
        for xx in range(max(0, x), min(x + w, W)):
            total += 1
            p = pa[xx, yy]
            q = pb[xx, yy]
            if abs(p[0] - q[0]) > THRESHOLD or abs(p[1] - q[1]) > THRESHOLD or abs(p[2] - q[2]) > THRESHOLD:
                diff += 1
    pct = 0.0 if total == 0 else 100.0 * diff / total
    print(f"region {x},{y},{w},{h} differing={diff}/{total} ({pct:.2f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
