# Measure the text advance difference (#240).
#
# React's Chromium draws the VARIABLE font (the woff2 the web app ships) and
# interpolates the wght axis, so its advances are fractional. The GPUI editor
# embeds STATIC instances made with fontTools' instantiateVariableFont, whose
# advances are whole font units.
#
# This script measures both, from the very files the two editors use:
#   - the variable font's advance at a weight (base hmtx + the HVAR delta),
#   - the static instance's advance (its own hmtx),
# and reports the per-glyph difference.
#
# Usage: python tools/ui-capture/font-advance.py
import json
import os
import sys

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import (
    NormalizedAxisLimits,
    NormalizedAxisTripleAndDistances,
    instantiateItemVariationStore,
)

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FONT_DIR = os.path.join(ROOT, "app", "gpui", "assets", "fonts")

WORDS = [
    "Nothing",
    "is",
    "running",
    "Nothing is running",
    "Layout",
    "Effects",
    "Devices",
    "Displays",
    "Fans",
    "Settings",
    "Updates",
    "Peripherals",
]

WEIGHTS = ((400, "schibsted-grotesk-400.ttf"), (500, "schibsted-grotesk-500.ttf"),
           (600, "schibsted-grotesk-600.ttf"), (650, "schibsted-grotesk-650.ttf"),
           (700, "schibsted-grotesk-700.ttf"))


def cmap_map(font):
    best = None
    for table in font["cmap"].tables:
        if table.isUnicode():
            if best is None or table.platformID == 3:
                best = table.cmap
    return best


def static_advances(path, words):
    """The advance of each word from the font's own hmtx (whole font units)."""
    font = TTFont(path, fontNumber=0, lazy=False)
    cmap = cmap_map(font)
    upem = font["head"].unitsPerEm
    hmtx = font["hmtx"]
    out = {}
    for w in words:
        total = 0.0
        per = []
        for ch in w:
            gname = cmap.get(ord(ch))
            adv = hmtx[gname][0] if gname else 0
            total += adv
            per.append((ch, gname, adv))
        out[w] = {"units": total, "px": total * 1000.0 / upem, "per_glyph": per, "upem": upem}
    font.close()
    return out


def variable_advances_at(var_path, weight, words):
    """The advance of each word in the VARIABLE font at `weight`: the base hmtx
    advance plus the HVAR delta at that location (fractional, as a browser
    shapes it)."""
    font = TTFont(var_path, fontNumber=0, lazy=False)
    cmap = cmap_map(font)
    upem = font["head"].unitsPerEm
    hmtx = font["hmtx"]
    fvar_axes = font["fvar"].axes
    axis = next(a for a in fvar_axes if a.axisTag == "wght")
    norm = (weight - axis.minValue) / (axis.maxValue - axis.minValue)
    limits = NormalizedAxisLimits(
        {"wght": NormalizedAxisTripleAndDistances(norm, norm, norm)}
    )
    hvar = font["HVAR"].table
    deltas = instantiateItemVariationStore(hvar.VarStore, fvar_axes, limits)
    adv_map = hvar.AdvWidthMap.mapping if hvar.AdvWidthMap else {}
    out = {}
    for w in words:
        total = 0.0
        per = []
        for ch in w:
            gname = cmap.get(ord(ch))
            if gname is None:
                per.append((ch, None, None, None))
                continue
            base = hmtx[gname][0]
            var_idx = adv_map.get(gname)
            delta = deltas.get(var_idx, 0.0) if var_idx is not None else 0.0
            total += base + delta
            per.append((ch, gname, base, delta))
        out[w] = {"units": total, "px": total * 1000.0 / upem, "per_glyph": per, "upem": upem}
    font.close()
    return out


def main():
    var_path = os.path.join(FONT_DIR, "schibsted-grotesk-variable.woff2")
    if not os.path.exists(var_path):
        print("missing", var_path)
        return 1
    report = {"words": WORDS, "weights": {}}
    for weight, static_name in WEIGHTS:
        static_path = os.path.join(FONT_DIR, static_name)
        var = variable_advances_at(var_path, weight, WORDS)
        stat = static_advances(static_path, WORDS) if os.path.exists(static_path) else None
        report["weights"][str(weight)] = {"variable": var, "static": stat}
        print(f"=== wght {weight} ===")
        for w in WORDS:
            vpx = var[w]["px"]
            line = f"  {w!r:22} variable {vpx:9.4f} px"
            if stat is not None:
                spx = stat[w]["px"]
                line += f"   static {spx:9.4f} px   delta {vpx - spx:+8.4f} px"
            print(line)
        if stat is not None:
            print("  per glyph (variable: base+HVAR delta  |  static: hmtx):")
            for w in WORDS:
                parts = []
                for (ch, gname, base, delta), (_c2, _g2, sadv) in zip(var[w]["per_glyph"], stat[w]["per_glyph"]):
                    parts.append(f"{ch}:{base}{delta:+.3f}/{sadv}")
                print(f"    {w!r:22} " + "  ".join(parts))
    out = os.path.join(ROOT, ".build", "ui-capture", "240-font-advance.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(report, fh, indent=2)
    print("wrote", out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
