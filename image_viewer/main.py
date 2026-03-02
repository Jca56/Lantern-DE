#!/usr/bin/env python3
"""FOX Image Viewer — A simple, fast image viewer for KDE Plasma / X11."""

import sys
import os

# Resolve the app directory (supports both installed and dev usage)
APP_DIR = os.path.dirname(os.path.abspath(__file__))
if APP_DIR not in sys.path:
    sys.path.insert(0, APP_DIR)

from PyQt6.QtWidgets import QApplication
from viewer.main_window import MainWindow


def main():
    app = QApplication(sys.argv)
    app.setApplicationName("FOX Image Viewer")
    app.setOrganizationName("FOX-DE")
    app.setDesktopFileName("fox-image-viewer")

    # Follow system theme (KDE Breeze)
    app.setStyle("Fusion")

    window = MainWindow()
    window.show()

    # If a file was passed as argument, open it
    if len(sys.argv) > 1:
        window.open_image(sys.argv[1])

    sys.exit(app.exec())


if __name__ == "__main__":
    main()
