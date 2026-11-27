"""Visual pass for the ROYUAN MonsGeek keyboard (USB 3151:4015), issue #10.

Two jobs, both on the REAL board through the vendor's live 0x0F frames (no flash write):

  python -I tools/kbd-map.py map [--out kbd-slot-map.csv] [--from 0] [--to 125]
      Lights ONE slot at a time (white, everything else black) and waits. Type the name of the
      key that lit and press Enter (blank = no key lit or unclear, b = back one, q = quit). The
      answers are written to the CSV after every step, so a stopped run keeps what it has and a
      new run with --from carries on. This settles the key-to-slot map (royuan.rs key_slot).

  python -I tools/kbd-map.py flicker [--rate] [--blind] [--seconds 5]
      Runs each flicker variant below for 5 s, prints which is which, and (with --rate) asks
      for a score after each: 0 steady, 1 slight, 2 clear, 3 strobing. --blind hides the label
      until the end.

  python -I tools/kbd-map.py selftest
      No hardware: checks the frame bytes against the golden bytes the Rust tests pin.

Both jobs pause canvas-rgb's output first (`canvas-rgb-cli output pause`) and ALWAYS resume it at
exit (try/finally), whatever happens in between. Needs the `hidapi` Python package (`import hid`).
Pass --cli PATH when canvas-rgb-cli is not on PATH. Pass --send-direct to send canvas-rgb's
direct-mode report first (only when the board is not in canvas-rgb's mode; the scripts that
proved 0x0F did not send it).

Frame layout, the same as crates/canvas-adapters/src/royuan.rs `sync_color_reports`: 7 reports,
`00 0F 01 <page> 38 <last> 00 00 <ck>` then 56 RGB bytes, ck = 255 - (sum(b1..b7) & 255), the
length byte 56 on every page, last = 1 on page 6, 126 slots, slot order column-major from the
bottom row.
"""
import argparse
import csv
import os
import random
import shutil
import subprocess
import sys
import threading
import time

VID, PID = 0x3151, 0x4015
SLOTS = 126
PAGES = 7
PAGE_DATA = 56
DIRECT_MODE = bytes([0, 0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]) + bytes(55)


def frame_reports(colors):
    """The seven 65-byte reports of one frame. `colors` is a list of (r, g, b), SLOTS long or shorter."""
    payload = bytearray(PAGES * PAGE_DATA)
    for slot, (r, g, b) in enumerate(colors[:SLOTS]):
        payload[slot * 3:slot * 3 + 3] = bytes((r, g, b))
    reports = []
    for page in range(PAGES):
        last = 1 if page == PAGES - 1 else 0
        head = [0x0F, 0x01, page, PAGE_DATA, last, 0, 0]
        head.append(255 - (sum(head) & 255))
        body = bytes(head) + bytes(payload[page * PAGE_DATA:(page + 1) * PAGE_DATA])
        reports.append(bytes([0]) + body)
    return reports


def selftest():
    """Golden bytes, the same ones royuan.rs pins in a_live_frame_is_seven_exact_reports_for_a_known_frame."""
    colors = [(0, 0, 0)] * SLOTS
    colors[0] = (1, 2, 3)
    colors[9] = (255, 0, 0)
    colors[125] = (7, 8, 9)
    reports = frame_reports(colors)
    assert len(reports) == 7
    for page, r in enumerate(reports):
        last = 1 if page == 6 else 0
        assert len(r) == 65, page
        assert r[:9] == bytes([0, 0x0F, 0x01, page, 56, last, 0, 0, 0xB7 - page - last]), page
    assert reports[6][8] == 0xB0
    assert reports[0][9:12] == bytes([1, 2, 3])
    assert reports[0][9 + 27:9 + 30] == bytes([255, 0, 0])
    assert reports[6][9 + 39:9 + 42] == bytes([7, 8, 9])
    assert not any(reports[6][9 + 42:])
    print("selftest ok: 7 reports, checksums 0xB7..0xB2 then 0xB0, length byte 56 on every page")


def open_board():
    import hid  # imported late so `selftest` runs without hidapi
    path = next(
        (d["path"] for d in hid.enumerate(VID, PID)
         if d["interface_number"] == 2 and d["usage_page"] == 0xFFFF),
        None,
    )
    if path is None:
        sys.exit("the keyboard's vendor interface (3151:4015, interface 2) is not there")
    dev = hid.device()
    dev.open_path(path)
    return dev


def send_frame(dev, colors):
    for report in frame_reports(colors):
        dev.send_feature_report(report)


def black():
    return [(0, 0, 0)] * SLOTS


class Stream:
    """Sends `colors` to the board in a thread until stopped: one frame alone shows nothing."""

    def __init__(self, dev, colors, interval=0.033):
        self.dev, self.colors, self.interval = dev, colors, interval
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)

    def run(self):
        while not self.stop.is_set():
            started = time.perf_counter()
            send_frame(self.dev, self.colors)
            self.stop.wait(max(0.0, self.interval - (time.perf_counter() - started)))

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join()


def cli_path(args):
    path = shutil.which(args.cli or "canvas-rgb-cli")
    if not path:
        sys.exit("canvas-rgb-cli was not found (not on PATH, or --cli is wrong): the board is not touched")
    return path


def run_cli(path, *words):
    return subprocess.run([path, *words], capture_output=True, text=True, timeout=30)


def map_slots(dev, args):
    rows = {}
    if os.path.exists(args.out):
        with open(args.out, newline="") as f:
            rows = {int(r["slot"]): r["key"] for r in csv.DictReader(f)}
    slot = args.first
    while slot <= args.last:
        colors = black()
        colors[slot] = (255, 255, 255)
        print(f"\nslot {slot}  (column {slot // 6}, {slot % 6} up from the bottom row)"
              f"   known: {rows.get(slot, '-')}")
        with Stream(dev, colors):
            answer = input("  the key that lit (blank = none, b = back, q = quit): ").strip()
        if answer.lower() == "q":
            break
        if answer.lower() == "b":
            slot = max(args.first, slot - 1)
            continue
        rows[slot] = answer
        with open(args.out, "w", newline="") as f:
            writer = csv.writer(f)
            writer.writerow(["slot", "key"])
            for s in sorted(rows):
                writer.writerow([s, rows[s]])
        slot += 1
    print(f"\n{len(rows)} slots recorded in {args.out}")


def bands():
    """A fixed picture: four colour bands across the columns."""
    palette = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255)]
    return [palette[min(3, (slot // 6) // 5)] for slot in range(SLOTS)]


def scrolled(step):
    """The same bands moved `step` columns: a frame that changes."""
    colors = bands()
    shift = (step % 21) * 6
    return colors[shift:] + colors[:shift]


# (label, interval ms, skip identical frames, keepalive ms or None, animate)
VARIANTS = [
    ("stream every 16 ms, no skip", 16, False, None, False),
    ("stream every 33 ms, no skip", 33, False, None, False),
    ("stream every 66 ms, no skip", 66, False, None, False),
    ("stream every 100 ms, no skip", 100, False, None, False),
    ("33 ms, skip identical, keepalive 100 ms (the adapter default)", 33, True, 100, False),
    ("33 ms, skip identical, keepalive 250 ms (does it stay lit?)", 33, True, 250, False),
    ("33 ms, skip identical, keepalive 500 ms (does it stay lit?)", 33, True, 500, False),
    ("33 ms, skip identical, NO keepalive (one frame, then nothing)", 33, True, None, False),
    ("moving bands, 33 ms, skip identical, keepalive 100 ms", 33, True, 100, True),
    ("moving bands, 100 ms, skip identical, keepalive 100 ms", 100, True, 100, True),
]


def run_variant(dev, variant, seconds):
    _, interval_ms, skip, keepalive_ms, animate = variant
    interval = interval_ms / 1000.0
    started = time.perf_counter()
    last_payload, last_at, sent = None, None, 0
    while time.perf_counter() - started < seconds:
        now = time.perf_counter()
        colors = scrolled(int((now - started) * 4)) if animate else bands()
        same = last_payload == colors
        due = last_at is None or (
            not skip
            or (not same)
            or (keepalive_ms is not None and now - last_at >= keepalive_ms / 1000.0)
        )
        if due and (last_at is None or now - last_at >= interval):
            send_frame(dev, colors)
            last_payload, last_at, sent = colors, time.perf_counter(), sent + 1
        time.sleep(0.004)
    send_frame(dev, black())
    return sent


def flicker(dev, args):
    order = list(range(len(VARIANTS)))
    if args.blind:
        random.shuffle(order)
    results = []
    for n, index in enumerate(order, 1):
        variant = VARIANTS[index]
        name = chr(ord("A") + n - 1) if args.blind else variant[0]
        input(f"\nvariant {n}/{len(order)}: {name}\n  press Enter to start {args.seconds:g} s ")
        sent = run_variant(dev, variant, args.seconds)
        print(f"  done: {sent} frames sent ({sent / args.seconds:.1f} per second)")
        score = input("  flicker 0 steady, 1 slight, 2 clear, 3 strobing (blank = skip): ").strip() if args.rate else ""
        results.append((name, variant[0], sent, score))
        time.sleep(1.0)
    print("\nwhich is which:")
    for name, label, sent, score in results:
        print(f"  {name}: {label}  [{sent} frames]  score {score or '-'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("job", choices=["map", "flicker", "selftest"])
    parser.add_argument("--cli", help="path to canvas-rgb-cli")
    parser.add_argument("--out", default="kbd-slot-map.csv")
    parser.add_argument("--from", dest="first", type=int, default=0)
    parser.add_argument("--to", dest="last", type=int, default=SLOTS - 1)
    parser.add_argument("--seconds", type=float, default=5.0)
    parser.add_argument("--rate", action="store_true")
    parser.add_argument("--blind", action="store_true")
    parser.add_argument("--send-direct", action="store_true")
    args = parser.parse_args()
    if args.job == "selftest":
        selftest()
        return
    if not (0 <= args.first <= args.last < SLOTS):
        sys.exit(f"--from/--to must satisfy 0 <= from <= to < {SLOTS}")

    cli = cli_path(args)
    paused = run_cli(cli, "output", "pause")
    if paused.returncode != 0:
        # Not paused: the daemon may still be driving the board, so the board is not touched.
        run_cli(cli, "output", "resume")
        sys.exit(f"could not pause canvas-rgb output ({paused.stderr.strip() or paused.stdout.strip()}); nothing was sent")
    dev = None
    try:
        dev = open_board()
        if args.send_direct:
            dev.send_feature_report(DIRECT_MODE)
        (map_slots if args.job == "map" else flicker)(dev, args)
    finally:
        try:
            if dev is not None:
                with Stream(dev, black()):
                    time.sleep(0.3)
                dev.close()
        finally:
            resumed = run_cli(cli, "output", "resume")
            if resumed.returncode != 0:
                print("CANVAS-RGB OUTPUT IS STILL PAUSED: run `canvas-rgb-cli output resume`", file=sys.stderr)
            else:
                print("canvas-rgb output resumed")


if __name__ == "__main__":
    main()
