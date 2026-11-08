# The kerning the font has, and what it is worth (#240).
#
#   python tools/ui-capture/kern-advance.py
#
# GPUI's text_width comes out wider than the browser's for the same word. The
# font's own advances are the same in both (see font-advance.py), so the
# difference is the kerning: Chromium applies the GPOS `kern` feature, and
# this script measures how much that is worth per word.
import json
import os
import sys

from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FONT_DIR = os.path.join(ROOT, "app", "gpui", "assets", "fonts")

WORDS = ["Nothing", "is", "running", "Nothing is running", "Layout", "Effects",
         "Devices", "Displays", "Fans", "Settings", "Updates", "Peripherals"]

# The browser's own measurements (canvas measureText at 14 px, weight 600,
# Schibsted Grotesk Variable) from tools/ui-capture/probe-chrome.mjs.
BROWSER = {
    "Nothing": 52.60,
    "is": 11.15,
    "running": 51.38,
    "Nothing is running": 121.49,
    "Layout": 45.38,
    "Effects": 46.57,
    "Devices": 53.68,
    "Displays": 56.93,
    "Settings": 54.84,
    "Updates": 55.76,
    "Peripherals": 77.22,
}

SIZE = 14.0


def cmap_map(font):
    best = None
    for table in font["cmap"].tables:
        if table.isUnicode():
            if best is None or table.platformID == 3:
                best = table.cmap
    return best


def kern_pairs(font):
    """The `kern` feature's pair adjustments: (left, right) -> x advance."""
    if "GPOS" not in font:
        return {}
    gpos = font["GPOS"].table
    pairs = {}
    for record in gpos.FeatureList.FeatureRecord:
        if record.FeatureTag != "kern":
            continue
        for lookup_index in record.Feature.LookupListIndex:
            lookup = gpos.LookupList.Lookup[lookup_index]
            for sub in lookup.SubTable:
                if getattr(sub, "Format", None) == 1:
                    for first, pairset in zip(sub.Coverage.glyphs, sub.PairSet):
                        for pairvalue in pairset.PairValueRecord:
                            pairs[(first, pairvalue.SecondGlyph)] = pairvalue.Value1.XAdvance
                elif getattr(sub, "Format", None) == 2:
                    class1 = sub.ClassDef1.classDefs
                    class2 = sub.ClassDef2.classDefs
                    cov = sub.Coverage.glyphs
                    for i, g1 in enumerate(cov):
                        c1 = class1.get(g1, 0)
                        for j, g2 in enumerate(cov):
                            c2 = class2.get(g2, 0)
                            rec = sub.Class1Record[c1].Class2Record[c2]
                            if rec.Value1 and rec.Value1.XAdvance:
                                pairs[(g1, g2)] = rec.Value1.XAdvance
    return pairs


def main():
    path = os.path.join(FONT_DIR, "schibsted-grotesk-600.ttf")
    font = TTFont(path, fontNumber=0, lazy=False)
    cmap = cmap_map(font)
    upem = font["head"].unitsPerEm
    hmtx = font["hmtx"]
    pairs = kern_pairs(font)
    report = {"size_px": SIZE, "words": {}}
    print(f"{FONT_DIR}  upem {upem}  kern pairs {len(pairs)}")
    print(f"{'word':22} {'raw px':>9} {'kern px':>9} {'raw-kern':>9} {'browser':>9} {'delta':>8}")
    for w in WORDS:
        names = [cmap.get(ord(c)) for c in w]
        raw = sum(hmtx[n][0] for n in names if n)
        kern = 0
        detail = []
        for a, b in zip(names, names[1:]):
            v = pairs.get((a, b))
            if v:
                kern += v
                detail.append(f"{a}|{b}={v}")
        raw_px = raw * SIZE / upem
        kern_px = kern * SIZE / upem
        shaped_px = raw_px - kern_px
        browser = BROWSER.get(w)
        delta = (shaped_px - browser) if browser is not None else None
        report["words"][w] = {
            "raw_units": raw, "kern_units": kern, "raw_px": raw_px,
            "shaped_px": shaped_px, "browser_px": browser, "delta_px": delta,
            "pairs": detail,
        }
        print(f"{w!r:22} {raw_px:9.4f} {kern_px:9.4f} {shaped_px:9.4f} "
              f"{(browser if browser is not None else float('nan')):9.4f} "
              f"{(delta if delta is not None else float('nan')):8.4f}")
    out = os.path.join(ROOT, ".build", "ui-capture", "240-kern-advance.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(report, fh, indent=2)
    print("wrote", out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
