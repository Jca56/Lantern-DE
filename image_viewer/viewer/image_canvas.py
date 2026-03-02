"""Core image display widget with zoom, pan, and rotation support."""

from PyQt6.QtWidgets import QGraphicsView, QGraphicsScene, QGraphicsPixmapItem
from PyQt6.QtGui import QPixmap, QTransform, QPainter, QWheelEvent, QMouseEvent
from PyQt6.QtCore import Qt, QRectF, pyqtSignal


class ImageCanvas(QGraphicsView):
    """A zoomable, pannable image display widget."""

    zoom_changed = pyqtSignal(float)  # emits zoom percentage

    MIN_ZOOM = 0.05
    MAX_ZOOM = 20.0
    ZOOM_STEP = 1.15

    def __init__(self, parent=None):
        super().__init__(parent)
        self._scene = QGraphicsScene(self)
        self.setScene(self._scene)

        self._pixmap_item: QGraphicsPixmapItem | None = None
        self._zoom_factor: float = 1.0
        self._rotation: int = 0  # degrees (multiples of 90)
        self._panning = False
        self._pan_start = None

        # Rendering quality
        self.setRenderHints(
            QPainter.RenderHint.Antialiasing
            | QPainter.RenderHint.SmoothPixmapTransform
        )
        self.setDragMode(QGraphicsView.DragMode.NoDrag)
        self.setTransformationAnchor(QGraphicsView.ViewportAnchor.AnchorUnderMouse)
        self.setResizeAnchor(QGraphicsView.ViewportAnchor.AnchorViewCenter)
        self.setViewportUpdateMode(QGraphicsView.ViewportUpdateMode.FullViewportUpdate)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)

        # Dark grey background
        self.setStyleSheet("QGraphicsView { background-color: #2b2b2b; border: none; }")

    # ── Public API ───────────────────────────────────────────────

    def load_pixmap(self, pixmap: QPixmap) -> None:
        """Display a new image, fitting it to the viewport."""
        self._scene.clear()
        self._pixmap_item = self._scene.addPixmap(pixmap)
        self._pixmap_item.setTransformationMode(Qt.TransformationMode.SmoothTransformation)
        self._scene.setSceneRect(QRectF(pixmap.rect()))
        self._rotation = 0
        self.fit_to_view()

    def has_image(self) -> bool:
        return self._pixmap_item is not None

    def fit_to_view(self) -> None:
        """Fit the image to the viewport while keeping aspect ratio."""
        if not self._pixmap_item:
            return
        self.resetTransform()
        self._zoom_factor = 1.0
        self.fitInView(self._scene.sceneRect(), Qt.AspectRatioMode.KeepAspectRatio)
        # Calculate actual zoom factor after fit
        transform = self.transform()
        self._zoom_factor = transform.m11()
        self._apply_rotation()
        self.zoom_changed.emit(self._zoom_factor * 100)

    def zoom_original(self) -> None:
        """Show image at 100% (1:1 pixel) zoom."""
        if not self._pixmap_item:
            return
        self.resetTransform()
        self._zoom_factor = 1.0
        self._apply_rotation()
        self.zoom_changed.emit(100.0)

    def zoom_in(self) -> None:
        self._apply_zoom(self.ZOOM_STEP)

    def zoom_out(self) -> None:
        self._apply_zoom(1.0 / self.ZOOM_STEP)

    def rotate_cw(self) -> None:
        """Rotate 90° clockwise."""
        self._rotation = (self._rotation + 90) % 360
        self._rebuild_transform()

    def rotate_ccw(self) -> None:
        """Rotate 90° counter-clockwise."""
        self._rotation = (self._rotation - 90) % 360
        self._rebuild_transform()

    # ── Internal ─────────────────────────────────────────────────

    def _apply_zoom(self, factor: float) -> None:
        new_zoom = self._zoom_factor * factor
        if new_zoom < self.MIN_ZOOM or new_zoom > self.MAX_ZOOM:
            return
        self._zoom_factor = new_zoom
        self._rebuild_transform()

    def _rebuild_transform(self) -> None:
        t = QTransform()
        t.scale(self._zoom_factor, self._zoom_factor)
        t.rotate(self._rotation)
        self.setTransform(t)
        self.zoom_changed.emit(self._zoom_factor * 100)

    def _apply_rotation(self) -> None:
        if self._rotation:
            self.rotate(self._rotation)

    # ── Events ───────────────────────────────────────────────────

    def wheelEvent(self, event: QWheelEvent) -> None:
        if event.angleDelta().y() > 0:
            self._apply_zoom(self.ZOOM_STEP)
        else:
            self._apply_zoom(1.0 / self.ZOOM_STEP)

    def mousePressEvent(self, event: QMouseEvent) -> None:
        if event.button() == Qt.MouseButton.MiddleButton or (
            event.button() == Qt.MouseButton.LeftButton
            and event.modifiers() & Qt.KeyboardModifier.ControlModifier
        ):
            self._panning = True
            self._pan_start = event.position().toPoint()
            self.setCursor(Qt.CursorShape.ClosedHandCursor)
            event.accept()
        else:
            super().mousePressEvent(event)

    def mouseMoveEvent(self, event: QMouseEvent) -> None:
        if self._panning and self._pan_start is not None:
            delta = event.position().toPoint() - self._pan_start
            self._pan_start = event.position().toPoint()
            self.horizontalScrollBar().setValue(
                self.horizontalScrollBar().value() - delta.x()
            )
            self.verticalScrollBar().setValue(
                self.verticalScrollBar().value() - delta.y()
            )
            event.accept()
        else:
            super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event: QMouseEvent) -> None:
        if self._panning:
            self._panning = False
            self.setCursor(Qt.CursorShape.ArrowCursor)
            event.accept()
        else:
            super().mouseReleaseEvent(event)

    def mouseDoubleClickEvent(self, event: QMouseEvent) -> None:
        """Double-click to fit image to view."""
        if event.button() == Qt.MouseButton.LeftButton:
            self.fit_to_view()
        else:
            super().mouseDoubleClickEvent(event)
