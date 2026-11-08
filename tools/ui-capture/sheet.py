# A contact sheet of the chrome: React's strip above GPUI's, for every page
# (#240). One image to look at, instead of fourteen.
#
#   python tools/ui-capture/sheet.py strips 0 0 390 48 out.png
#   python tools/ui-capture/sheet.py docks 0 779 390 65 out.png
import os
import sys

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CAP = os.path.join(ROOT, ".build", "ui-capture")
PAGES = ["layout", "effects", "devices", "displays", "fans", "settings", "updates"]


def main():
    if len(sys.argv) < 6:
        print(__doc__)
        return 1
    what = sys.argv[1]
    x, y, w, h = (int(v) for v in sys.argv[2:6])
    out = sys.argv[6] if len(sys.argv) > 6 else os.path.join(CAP, f"240-sheet-{what}.png")
    scale = 2
    label_w = 90
    rows = []
    for page in PAGES:
        react = Image.open(os.path.join(CAP, f"{page}-react-390x844-dark.png")).convert("RGB").crop((x, y, x + w, y + h))
        gpui = Image.open(os.path.join(CAP, f"{page}-gpui-390x844-dark.png")).convert("RGB").crop((x, y, x + w, y + h))
        rows.append((page, react, gpui))
    cell_w = w * scale
    cell_h = h * scale
    pad = 4
    sheet = Image.new("RGB", (label_w + cell_w * 2 + pad * 3, (cell_h * 2 + pad * 3) * len(rows)), (255, 0, 255))
    draw = ImageDraw.Draw(sheet)
    for i, (page, react, gpui) in enumerate(rows):
        top = i * (cell_h * 2 + pad * 3)
        draw.text((4, top + 4), f"{page}\nreact", fill=(255, 255, 255))
        draw.text((4, top + cell_h + pad + 4), "gpui", fill=(255, 255, 255))
        sheet.paste(react.resize((cell_w, cell_h), Image.NEAREST), (label_w, top))
        sheet.paste(gpui.resize((cell_w, cell_h), Image.NEAREST), (label_w, top + cell_h + pad))
    sheet.save(out)
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
