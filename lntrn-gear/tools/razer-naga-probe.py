#!/usr/bin/env python3
"""Read-only probe: print what each Naga side button emits RIGHT NOW.

Writes nothing. For every key press it shows the evdev keycode (decoded to a
name) and the HID scancode (what hwdb's KEYBOARD_KEY_<scan> would match).
Run it, press the side buttons in order, then Ctrl-C:

    python3 tools/razer-naga-probe.py
"""

import glob
import os
import select
import struct
import sys

EV_FORMAT = "@llHHi"
EV_SIZE = struct.calcsize(EV_FORMAT)
EV_KEY, EV_MSC, MSC_SCAN = 0x01, 0x04, 0x04

# evdev keycode -> name, just the codes these buttons could plausibly emit.
NAMES = {
    79: "KP1", 80: "KP2", 81: "KP3", 75: "KP4", 76: "KP5",
    77: "KP6", 71: "KP7", 72: "KP8", 73: "KP9", 82: "KP0",
    74: "KPMINUS", 78: "KPPLUS", 96: "KPENTER", 83: "KPDOT",
    98: "KPSLASH", 55: "KPASTERISK",
    107: "END", 108: "DOWN", 109: "PAGEDOWN", 105: "LEFT",
    106: "RIGHT", 102: "HOME", 103: "UP", 104: "PAGEUP",
    110: "INSERT", 111: "DELETE",
    12: "MINUS (main row)", 13: "EQUAL (main row)",
    2: "1", 3: "2", 4: "3", 5: "4", 6: "5",
    7: "6", 8: "7", 9: "8", 10: "9", 11: "0",
}


def main():
    links = glob.glob("/dev/input/by-id/*Razer*Naga*event-kbd*")
    nodes = sorted({os.path.realpath(p) for p in links})
    if not nodes:
        sys.exit("✗ No Razer Naga keyboard interface found — is the mouse plugged in?")

    fds = {}
    for n in nodes:
        try:
            fds[os.open(n, os.O_RDONLY | os.O_NONBLOCK)] = n
        except PermissionError:
            sys.exit(f"✗ Can't read {n} — are you in the `input` group?")

    print(f"Listening on: {', '.join(fds.values())}")
    print("Press each side button (1..12). Ctrl-C when done.\n")
    print(f"  {'evdev code':>10}  {'name':<18}  {'HID scancode':>12}")
    print(f"  {'-'*10}  {'-'*18}  {'-'*12}")

    poller = select.poll()
    for fd in fds:
        poller.register(fd, select.POLLIN)

    last_scan = None
    try:
        while True:
            for fd, _ in poller.poll():
                data = os.read(fd, EV_SIZE * 64)
                for off in range(0, len(data) - EV_SIZE + 1, EV_SIZE):
                    _, _, etype, code, value = struct.unpack(EV_FORMAT, data[off:off + EV_SIZE])
                    if etype == EV_MSC and code == MSC_SCAN:
                        last_scan = value & 0xffffffff
                    elif etype == EV_KEY and value == 1:  # press only
                        name = NAMES.get(code, "?")
                        scan = f"0x{last_scan:x}" if last_scan is not None else "(none)"
                        print(f"  {code:>10}  {name:<18}  {scan:>12}")
    except KeyboardInterrupt:
        print("\ndone.")


if __name__ == "__main__":
    main()
