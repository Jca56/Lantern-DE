# Lantern Spark SDK Specification

**Version:** 0.1.0-draft
**Status:** Draft
**Date:** 2026-03-03

---

## Overview

A **Spark** is a self-contained widget that runs in the Lantern DE appbar.
Each Spark is a separate process that communicates with the bar over a Unix
domain socket using a simple binary protocol. Sparks render their own pixels
to an RGBA buffer and send it to the bar for compositing.

This architecture gives Spark developers maximum freedom:
- Write in any language (Rust, C, Python, Go, JS, etc.)
- Use any rendering approach (GPU, software, web view, terminal)
- Crash without taking down the bar
- Hot-reload during development

---

## Terminology

| Term | Definition |
|---|---|
| **Bar** | The Lantern appbar process (`lntrn-bar`) |
| **Spark** | A widget process that renders into the bar |
| **Slot** | A positioned area in the bar where a Spark renders |
| **Manifest** | A TOML file describing a Spark's metadata and config |
| **Frame** | A single rendered RGBA pixel buffer from a Spark |

---

## Spark Lifecycle

```
1. Bar starts → reads config → knows which Sparks to load
2. Bar creates a Unix socket at $XDG_RUNTIME_DIR/lantern/sparks/<spark-id>.sock
3. Bar spawns the Spark process, passing the socket path as argv[1]
4. Spark connects to the socket
5. Bar sends Init message (bar height, assigned width, theme colors)
6. Spark sends Ready message (confirms preferred size)
7. Bar sends Render request
8. Spark sends Frame (RGBA pixel buffer)
9. Bar composites the frame into the bar surface
10. Loop: bar sends events (click, hover, resize) → Spark sends frames
11. Bar sends Shutdown → Spark exits cleanly
```

---

## Socket Protocol

All messages are length-prefixed binary frames:

```
┌──────────┬──────────┬─────────────────────┐
│ Length    │ Type     │ Payload             │
│ 4 bytes  │ 1 byte   │ variable            │
│ u32 LE   │ u8       │ MessagePack or raw  │
└──────────┴──────────┴─────────────────────┘
```

- **Length** includes Type + Payload (not the length field itself)
- **Payload** is MessagePack-encoded for structured messages, raw bytes for pixel data
- Messages are not compressed (pixel buffers are already in shared memory, see below)

---

## Message Types

### Bar → Spark Messages

| Type ID | Name | Payload | Description |
|---|---|---|---|
| `0x01` | Init | `{ width: u32, height: u32, scale: f32, theme: Theme }` | Initial configuration |
| `0x02` | Resize | `{ width: u32, height: u32 }` | Slot size changed |
| `0x03` | Render | `{}` | Request a new frame |
| `0x04` | MouseEvent | `{ kind: str, x: f32, y: f32, button: u8 }` | Mouse interaction |
| `0x05` | ThemeChanged | `{ theme: Theme }` | Bar theme was updated |
| `0x06` | Shutdown | `{}` | Clean exit requested |

**MouseEvent kinds:** `"enter"`, `"leave"`, `"move"`, `"press"`, `"release"`, `"click"`
**Button values:** `0` = left, `1` = right, `2` = middle

### Spark → Bar Messages

| Type ID | Name | Payload | Description |
|---|---|---|---|
| `0x81` | Ready | `{ preferred_width: u32, min_width: u32, max_width: u32 }` | Spark is ready |
| `0x82` | Frame | `{ shm_name: str, width: u32, height: u32, stride: u32 }` | New pixel buffer |
| `0x83` | RequestSize | `{ width: u32 }` | Spark wants to resize |
| `0x84` | Tooltip | `{ text: str }` | Set hover tooltip |
| `0x85` | ContextMenu | `{ items: [MenuItem] }` | Request a context menu |
| `0x86` | Notification | `{ title: str, body: str, icon: str? }` | Send a notification |

---

## Pixel Buffer Format

Sparks render to shared memory (POSIX shm) for zero-copy transfer:

1. Spark creates a shared memory segment: `shm_open("/lantern-spark-<id>", ...)`
2. Spark renders RGBA pixels (8 bits per channel, 32 bits per pixel) into the segment
3. Spark sends a `Frame` message with the shm name and dimensions
4. Bar maps the shared memory and composites it
5. Bar unmaps after compositing (Spark retains ownership)

**Pixel format:** RGBA8888 (R at byte 0, A at byte 3), pre-multiplied alpha
**Origin:** Top-left corner
**Stride:** `width * 4` bytes per row (no padding)

This means a 200×56 Spark at 32bpp uses only ~44KB per frame.

For Sparks that don't need shared memory (simple, low-frequency updates),
a `FrameInline` message (type `0x87`) can send pixel data directly over the
socket. This is simpler but slower for large buffers.

| Type ID | Name | Payload | Description |
|---|---|---|---|
| `0x87` | FrameInline | `{ width: u32, height: u32, pixels: bytes }` | Inline pixel data |

---

## Theme Object

The bar sends theme colors so Sparks can match the DE aesthetic.
Sparks are free to ignore this and render however they want.

```json
{
    "bg": [28, 28, 28, 255],
    "surface": [39, 39, 39, 255],
    "surface_2": [51, 51, 51, 255],
    "text": [236, 236, 236, 255],
    "muted": [144, 144, 144, 255],
    "accent": [200, 134, 10, 255]
}
```

Each color is `[R, G, B, A]` with values 0–255.

---

## Spark Manifest

Every Spark has a `spark.toml` manifest file.

**Location:** `~/.local/share/lantern/sparks/<spark-id>/spark.toml`
**System:** `/usr/share/lantern/sparks/<spark-id>/spark.toml`

```toml
[spark]
id = "org.lantern.clock"
name = "Clock"
version = "1.0.0"
description = "Displays the current time and date"
author = "Lantern Project"
license = "MIT"

[display]
preferred_width = 120
min_width = 80
max_width = 200
# Where this Spark prefers to be placed (suggestion, user can override)
# "left" | "center" | "right"
default_position = "right"

[exec]
# Path relative to the Spark directory, or absolute
command = "lantern-spark-clock"
# Optional: working directory
# workdir = "."

[config]
# Optional: Spark-specific config schema
# The bar will pass config values at Init time
# format_24h = true
# show_seconds = false
# timezone = "local"
```

---

## Spark Directory Structure

```
~/.local/share/lantern/sparks/org.lantern.clock/
    spark.toml          # Manifest (required)
    lantern-spark-clock # Executable (or script)
    icons/              # Optional icon assets
        clock-16.png
        clock-32.png
    README.md           # Optional documentation
```

---

## Spark Installation

Sparks can be installed by:
1. **Manual:** Drop a directory into `~/.local/share/lantern/sparks/`
2. **System package:** Install to `/usr/share/lantern/sparks/` via pacman/apt
3. **Future:** A Spark store / registry (stretch goal)

The bar scans both directories on startup and presents available Sparks
in its settings UI.

---

## Bar Config (Spark Slots)

The bar's `appbar.json` config references installed Sparks:

```json
{
    "bar_position": "bottom",
    "height": 56,
    "sparks": [
        {
            "id": "org.lantern.clock",
            "position": 1.0,
            "config": {
                "format_24h": true,
                "show_seconds": false
            }
        },
        {
            "id": "org.lantern.systray",
            "position": 0.9
        },
        {
            "id": "com.example.weather",
            "position": 0.5,
            "config": {
                "city": "New York"
            }
        }
    ]
}
```

---

## Context Menu Items

Sparks can request context menu items that appear when the user right-clicks
on the Spark's area:

```json
{
    "items": [
        { "label": "Settings...", "action": "settings" },
        { "kind": "separator" },
        { "label": "12-Hour Format", "action": "toggle_24h", "checked": false },
        { "label": "Show Seconds", "action": "toggle_seconds", "checked": true },
        { "kind": "separator" },
        { "label": "Remove from bar", "action": "__remove" }
    ]
}
```

When the user clicks a context menu item, the bar sends a `MenuAction`
event (type `0x08`) with the `action` string back to the Spark.

| Type ID | Name | Payload | Description |
|---|---|---|---|
| `0x08` | MenuAction | `{ action: str }` | User clicked a context menu item |

Actions prefixed with `__` are handled by the bar itself:
- `__remove` — Remove this Spark from the bar
- `__move_left` / `__move_right` — Reorder

---

## Error Handling

- If a Spark crashes, the bar shows a placeholder in its slot (muted error icon)
- The bar will attempt to restart crashed Sparks up to 3 times with exponential backoff
- If a Spark doesn't send a Frame within 5 seconds of a Render request, the bar
  shows the last frame with a "not responding" indicator
- The bar logs all Spark errors to `$XDG_STATE_HOME/lantern/spark-errors.log`

---

## Example: Minimal Clock Spark (Rust)

```rust
// This is pseudocode showing the conceptual flow.
// A real lntrn-spark-sdk crate would wrap all the socket/shm boilerplate.

fn main() {
    let socket_path = std::env::args().nth(1).expect("socket path");
    let conn = SparkConnection::connect(&socket_path);

    // Wait for Init
    let init = conn.recv_init();
    let mut width = init.width;
    let mut height = init.height;

    // Tell the bar we're ready
    conn.send_ready(preferred_width: 120, min_width: 80, max_width: 200);

    loop {
        match conn.recv() {
            Message::Render => {
                let mut buf = PixelBuffer::new(width, height);
                let time = format_time();
                buf.draw_text(&time, init.theme.text, 16.0);
                conn.send_frame(&buf);
            }
            Message::Resize { w, h } => {
                width = w;
                height = h;
            }
            Message::MouseEvent { kind: "click", .. } => {
                // Toggle 24h format or something
            }
            Message::Shutdown => break,
            _ => {}
        }
    }
}
```

---

## Example: Minimal Spark (Python)

```python
#!/usr/bin/env python3
"""Minimal Spark example in Python — shows that any language works."""

import sys
from lantern_spark import SparkConnection, PixelBuffer

def main():
    conn = SparkConnection(sys.argv[1])
    init = conn.recv_init()
    conn.send_ready(preferred_width=100, min_width=60, max_width=160)

    while True:
        msg = conn.recv()
        if msg.type == "render":
            buf = PixelBuffer(init.width, init.height)
            buf.fill(init.theme["bg"])
            buf.draw_text("Hello!", x=8, y=20, color=init.theme["text"])
            conn.send_frame(buf)
        elif msg.type == "shutdown":
            break

if __name__ == "__main__":
    main()
```

---

## Future Considerations

- **Spark-to-Spark communication** — allow Sparks to discover and message each other
  (e.g., a media Spark tells a notification Spark about track changes)
- **Accessibility** — Sparks should expose accessibility info via AT-SPI2 protocol
- **Animations** — Bar could request frames at 60fps for animated Sparks,
  or Sparks could push frames proactively (needs flow control)
- **Wayland support** — When Lantern moves to Wayland, the bar becomes a
  layer-shell surface. Spark protocol stays the same (IPC + pixel buffers).
  Only the bar's own compositor integration changes.

---

## SDK Crate Roadmap

A `lantern-spark-sdk` Rust crate will be provided that handles:
- Socket connection and message parsing
- Shared memory pixel buffer management
- Theme color helpers
- Simple 2D drawing primitives (text, rect, circle, image)
- Convenience macros for Spark boilerplate

Third-party bindings planned:
- `lantern-spark-python` — Python package
- `lantern-spark-c` — C header + static library
