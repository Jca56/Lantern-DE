#!/usr/bin/env python3
"""Capture the Razer Naga Trinity side-grid scancodes and emit a udev hwdb
keymap that remaps the 12 side buttons to the numeric keypad.

Why hwdb: the side grid sends ordinary *keyboard* keys (the number row by
default), so we can remap them at the kernel level — no daemon, no extra
packages, works on Wayland, survives reboots. This is the OS-layer step;
lighting/DPI will come from the lntrn-gear Razer driver later.

Run it (no root needed to capture — you're in the `input` group):

    python3 tools/razer-naga-numpad.py

Press each side button when prompted (physical labels 1..12). It writes
udev/61-razer-naga-numpad.hwdb next to this repo's existing udev rule and
prints the two sudo commands to activate it.

Edit TARGETS below if you want a different numpad layout for buttons 10-12
(e.g. kpdot, kpenter, kpasterisk, kpslash).
"""

import glob
import os
import select
import struct
import sys
from pathlib import Path

# Physical side button (1-indexed) -> hwdb keycode name (lowercase, no KEY_).
# 1-9 -> keypad 1-9, 10 -> keypad 0, 11 -> keypad minus, 12 -> keypad plus.
TARGETS = [
    "kp1", "kp2", "kp3",
    "kp4", "kp5", "kp6",
    "kp7", "kp8", "kp9",
    "kp0", "kpminus", "kpplus",
]

# Naga Trinity. Bus 0003 = USB; v1532 p0067 from `lsusb`. The hwdb match is
# by modalias prefix, so it targets only this device (not your Logitech kbd).
MODALIAS_MATCH = "evdev:input:b0003v1532p0067*"
OUT = Path(__file__).resolve().parent.parent / "udev" / "61-razer-naga-numpad.hwdb"

# struct input_event on 64-bit Linux: struct timeval (two longs) + u16 type
# + u16 code + s32 value = 24 bytes. Native byte order/alignment.
EV_FORMAT = "@llHHi"
EV_SIZE = struct.calcsize(EV_FORMAT)
EV_KEY, EV_MSC, MSC_SCAN = 0x01, 0x04, 0x04


def find_kbd_nodes():
    """The Naga's keyboard interfaces, resolved to /dev/input/eventN."""
    links = glob.glob("/dev/input/by-id/*Razer*Naga*event-kbd*")
    nodes = sorted({os.path.realpath(p) for p in links})
    if not nodes:
        sys.exit("✗ No Razer Naga keyboard interface found — is the mouse plugged in?")
    return nodes


def capture(nodes):
    """Open the kbd interfaces and record (scancode, keycode) for each of the
    12 buttons, one press at a time. Returns a list of dicts."""
    fds = {}
    for n in nodes:
        try:
            fds[os.open(n, os.O_RDONLY | os.O_NONBLOCK)] = n
        except PermissionError:
            sys.exit(f"✗ Can't read {n}. Are you in the `input` group? (id -nG)")

    print(f"Listening on: {', '.join(fds.values())}")
    print("Press each side button ONCE when prompted. Ctrl-C to abort.\n")

    results = []
    last_scan = None
    poller = select.poll()
    for fd in fds:
        poller.register(fd, select.POLLIN)

    try:
        for i, target in enumerate(TARGETS, start=1):
            print(f"  → press side button {i:>2}  (will map to {target}) ... ", end="", flush=True)
            captured = None
            while captured is None:
                for fd, _ in poller.poll():
                    data = os.read(fd, EV_SIZE * 64)
                    for off in range(0, len(data) - EV_SIZE + 1, EV_SIZE):
                        _, _, etype, code, value = struct.unpack(EV_FORMAT, data[off:off + EV_SIZE])
                        if etype == EV_MSC and code == MSC_SCAN:
                            last_scan = value & 0xffffffff
                        elif etype == EV_KEY and value == 1:  # key press
                            captured = {"button": i, "target": target,
                                        "scan": last_scan, "keycode": code}
                # wait for release so the next prompt starts clean
                if captured is not None:
                    _drain_until_release(poller, fds, captured["keycode"])
            if captured["scan"] is None:
                print("no scancode (MSC_SCAN) — hwdb can't remap this one ✗")
            else:
                print(f"scancode 0x{captured['scan']:x}, keycode {captured['keycode']} ✓")
            results.append(captured)
    finally:
        for fd in fds:
            os.close(fd)
    return results


def _drain_until_release(poller, fds, keycode):
    """Block until we see the release (value 0) for `keycode`, swallowing
    autorepeat so one physical press = one capture."""
    while True:
        for fd, _ in poller.poll():
            data = os.read(fd, EV_SIZE * 64)
            for off in range(0, len(data) - EV_SIZE + 1, EV_SIZE):
                _, _, etype, code, value = struct.unpack(EV_FORMAT, data[off:off + EV_SIZE])
                if etype == EV_KEY and code == keycode and value == 0:
                    return


def write_hwdb(results):
    usable = [r for r in results if r["scan"] is not None]
    if not usable:
        sys.exit("\n✗ None of the buttons emitted a scancode — hwdb won't work here.\n"
                 "  Fall back to keyd or input-remapper for these buttons.")

    lines = [
        "# Razer Naga Trinity (1532:0067) — side grid -> numeric keypad.",
        "# Generated by tools/razer-naga-numpad.py. Install:",
        "#   sudo mkdir -p /etc/udev/hwdb.d/    # may not exist yet",
        "#   sudo cp udev/61-razer-naga-numpad.hwdb /etc/udev/hwdb.d/",
        f"#   sudo {hwdb_update_cmd()}",
        "#   sudo udevadm trigger -s input    # or just replug the mouse",
        MODALIAS_MATCH,
    ]
    for r in usable:
        lines.append(f" KEYBOARD_KEY_{r['scan']:x}={r['target']}")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n")

    skipped = len(results) - len(usable)
    print(f"\n✓ Wrote {OUT}  ({len(usable)} buttons"
          + (f", {skipped} skipped — no scancode" if skipped else "") + ")")
    print("\nActivate it:")
    print(f"  sudo mkdir -p /etc/udev/hwdb.d/    # may not exist yet")
    print(f"  sudo cp {OUT} /etc/udev/hwdb.d/")
    print(f"  sudo {hwdb_update_cmd()}")
    print(f"  sudo udevadm trigger -s input    # or just replug the mouse")
    print("\nTest: open a text field and tap the side buttons — they should type")
    print("numpad digits. In WoW they'll bind as Numpad 1-9, 0, -, +.")


def hwdb_update_cmd():
    # systemd-udev uses `systemd-hwdb update`; eudev uses `udevadm hwdb --update`.
    for p in os.environ.get("PATH", "").split(":"):
        if os.path.exists(os.path.join(p, "systemd-hwdb")):
            return "systemd-hwdb update"
    return "udevadm hwdb --update"


if __name__ == "__main__":
    try:
        write_hwdb(capture(find_kbd_nodes()))
    except KeyboardInterrupt:
        sys.exit("\naborted.")
