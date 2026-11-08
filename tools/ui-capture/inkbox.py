# Ink boxes: the bounding box and count of ink pixels (any channel more than
# -T away from the panel background) in a region of two shots, so a text
# difference that is rasterisation only shows matching boxes. (#208)
#   python tools/ui-capture/inkbox.py react.png gpui.png 85 139 180 25 ffffff 40
import sys

from PIL import Image


def ink(path, box, bg, t):
    im = Image.open(path).convert("RGB")
    p = im.load()
    x0, y0, w, h = box
    xs, ys = [], []
    for y in range(y0, y0 + h):
        for x in range(x0, x0 + w):
            c = p[x, y]
            if max(abs(c[0] - bg[0]), abs(c[1] - bg[1]), abs(c[2] - bg[2])) > t:
                xs.append(x)
                ys.append(y)
    if not xs:
        return None
    return (min(xs), min(ys), max(xs), max(ys), len(xs))


def main():
    a, b = sys.argv[1], sys.argv[2]
    box = tuple(int(v) for v in sys.argv[3:7])
    bg = tuple(int(sys.argv[7][i:i + 2], 16) for i in (0, 2, 4))
    t = int(sys.argv[8]) if len(sys.argv) > 8 else 40
    print("react", ink(a, box, bg, t))
    print("gpui ", ink(b, box, bg, t))


main()
