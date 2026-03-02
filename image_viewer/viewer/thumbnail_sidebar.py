"""Thumbnail strip sidebar for browsing images in a directory."""

import os
from pathlib import Path

from PyQt6.QtWidgets import QListWidget, QListWidgetItem, QAbstractItemView
from PyQt6.QtGui import QIcon, QPixmap, QImage
from PyQt6.QtCore import Qt, QSize, pyqtSignal, QThread, pyqtSlot


# Supported image formats
IMAGE_EXTENSIONS = {
    ".png", ".jpg", ".jpeg", ".bmp", ".gif", ".webp",
    ".svg", ".tiff", ".tif", ".ico", ".pbm", ".pgm", ".ppm", ".xbm", ".xpm",
}


def is_image_file(path: str) -> bool:
    return Path(path).suffix.lower() in IMAGE_EXTENSIONS


class ThumbnailLoader(QThread):
    """Background thread that generates thumbnails."""

    thumbnail_ready = pyqtSignal(int, QIcon)  # (index, icon)

    def __init__(self, file_list: list[str], thumb_size: int = 128, parent=None):
        super().__init__(parent)
        self._file_list = file_list
        self._thumb_size = thumb_size
        self._abort = False

    def abort(self):
        self._abort = True

    def run(self):
        for i, path in enumerate(self._file_list):
            if self._abort:
                return
            try:
                img = QImage(path)
                if img.isNull():
                    continue
                scaled = img.scaled(
                    self._thumb_size,
                    self._thumb_size,
                    Qt.AspectRatioMode.KeepAspectRatio,
                    Qt.TransformationMode.SmoothTransformation,
                )
                icon = QIcon(QPixmap.fromImage(scaled))
                self.thumbnail_ready.emit(i, icon)
            except Exception:
                continue


class ThumbnailSidebar(QListWidget):
    """A vertical thumbnail strip for navigating images in a folder."""

    image_selected = pyqtSignal(str)  # emits the full file path

    THUMB_SIZE = 128

    def __init__(self, parent=None):
        super().__init__(parent)
        self._file_list: list[str] = []
        self._loader: ThumbnailLoader | None = None

        self.setViewMode(QListWidget.ViewMode.IconMode)
        self.setFlow(QListWidget.Flow.TopToBottom)
        self.setIconSize(QSize(self.THUMB_SIZE, self.THUMB_SIZE))
        self.setSpacing(6)
        self.setFixedWidth(self.THUMB_SIZE + 40)
        self.setWrapping(False)
        self.setResizeMode(QListWidget.ResizeMode.Adjust)
        self.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
        self.setStyleSheet("""
            QListWidget {
                background-color: #252525;
                border: none;
                border-right: 1px solid #3a3a3a;
            }
            QListWidget::item {
                padding: 4px;
                border-radius: 6px;
                color: #cccccc;
            }
            QListWidget::item:selected {
                background-color: #444444;
            }
            QListWidget::item:hover {
                background-color: #3a3a3a;
            }
        """)

        self.currentRowChanged.connect(self._on_row_changed)

    def load_directory(self, directory: str, select_file: str | None = None) -> None:
        """Scan a directory for images and populate the thumbnail list."""
        self._stop_loader()
        self.clear()

        dirpath = Path(directory)
        if not dirpath.is_dir():
            return

        self._file_list = sorted(
            [
                str(f)
                for f in dirpath.iterdir()
                if f.is_file() and is_image_file(str(f))
            ]
        )

        if not self._file_list:
            return

        # Add placeholder items
        for filepath in self._file_list:
            name = Path(filepath).name
            item = QListWidgetItem(name)
            item.setToolTip(filepath)
            item.setSizeHint(QSize(self.THUMB_SIZE + 20, self.THUMB_SIZE + 30))
            self.addItem(item)

        # Start background thumbnail loading
        self._loader = ThumbnailLoader(self._file_list, self.THUMB_SIZE, self)
        self._loader.thumbnail_ready.connect(self._on_thumbnail_ready)
        self._loader.start()

        # Select the target file
        if select_file and select_file in self._file_list:
            idx = self._file_list.index(select_file)
            self.setCurrentRow(idx)
        else:
            self.setCurrentRow(0)

    def select_file(self, filepath: str) -> None:
        """Select a file in the list by its path."""
        if filepath in self._file_list:
            idx = self._file_list.index(filepath)
            self.blockSignals(True)
            self.setCurrentRow(idx)
            self.blockSignals(False)

    def get_file_list(self) -> list[str]:
        return list(self._file_list)

    def current_file(self) -> str | None:
        row = self.currentRow()
        if 0 <= row < len(self._file_list):
            return self._file_list[row]
        return None

    def next_image(self) -> str | None:
        row = self.currentRow()
        if row + 1 < len(self._file_list):
            self.setCurrentRow(row + 1)
            return self._file_list[row + 1]
        return None

    def prev_image(self) -> str | None:
        row = self.currentRow()
        if row - 1 >= 0:
            self.setCurrentRow(row - 1)
            return self._file_list[row - 1]
        return None

    def _stop_loader(self):
        if self._loader and self._loader.isRunning():
            self._loader.abort()
            self._loader.wait(2000)
            self._loader = None

    @pyqtSlot(int, QIcon)
    def _on_thumbnail_ready(self, index: int, icon: QIcon):
        if 0 <= index < self.count():
            self.item(index).setIcon(icon)

    def _on_row_changed(self, row: int):
        if 0 <= row < len(self._file_list):
            self.image_selected.emit(self._file_list[row])
