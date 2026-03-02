# FOX Image Viewer

A simple, fast image viewer built with PyQt6 for **KDE Plasma on Arch Linux (X11)**.

Part of the **FOX-DE** project.

![Python](https://img.shields.io/badge/Python-3.11+-blue)
![Qt](https://img.shields.io/badge/Qt-PyQt6-green)
![Platform](https://img.shields.io/badge/Platform-Arch%20Linux%20%2F%20KDE%20Plasma-blue)

## Features

- **Smooth zoom** — scroll wheel, pinch-to-zoom, or keyboard shortcuts
- **Pan** — middle-click drag or Ctrl+click drag
- **Thumbnail sidebar** — browse all images in a folder with async thumbnail loading
- **Keyboard navigation** — arrow keys, Space/Backspace to move between images
- **Rotation** — 90° CW/CCW with R / Shift+R
- **Fullscreen** — F11 to toggle, Escape to exit
- **Drag & drop** — drop images or folders onto the window
- **Fit to view** — double-click or Ctrl+0 to auto-fit

## Install

```bash
# Make sure Python 3.11+ is installed
sudo pacman -S python python-pip

# Install dependencies
pip install -r requirements.txt

# Or install PyQt6 from Arch repos
sudo pacman -S python-pyqt6
```

## Run

```bash
# Launch the viewer
python main.py

# Open a specific image
python main.py /path/to/image.png

# Or make it executable
chmod +x main.py
./main.py
```

## Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+O` | Open image file |
| `Ctrl+Shift+O` | Open folder |
| `→` / `Space` | Next image |
| `←` / `Backspace` | Previous image |
| `Ctrl+=` | Zoom in |
| `Ctrl+-` | Zoom out |
| `Ctrl+0` | Fit to window |
| `Ctrl+1` | Original size (100%) |
| `R` | Rotate clockwise |
| `Shift+R` | Rotate counter-clockwise |
| `F11` | Toggle fullscreen |
| `F9` | Toggle sidebar |
| `Escape` | Exit fullscreen |
| `Ctrl+Q` | Quit |
| Middle-click drag | Pan |
| Scroll wheel | Zoom |
| Double-click | Fit to view |

## Project Structure

```
image_viewer/
├── main.py                  # Entry point
├── requirements.txt         # Python dependencies
├── README.md
└── viewer/
    ├── __init__.py
    ├── image_canvas.py      # Zoomable/pannable image display
    ├── thumbnail_sidebar.py # Folder thumbnail browser
    └── main_window.py       # Main window, menus, toolbar
```

## License

Part of FOX-DE.
