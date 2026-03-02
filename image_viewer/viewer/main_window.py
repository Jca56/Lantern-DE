"""Main application window with custom title bar, menus, and image display."""

import os
from pathlib import Path

from PyQt6.QtWidgets import (
    QMainWindow, QFileDialog, QLabel, QHBoxLayout, QWidget,
    QStatusBar, QSplitter, QMessageBox, QApplication,
    QPushButton, QMenuBar, QSizePolicy,
)
from PyQt6.QtGui import (
    QPixmap, QAction, QKeySequence, QIcon, QImageReader, QMouseEvent,
)
from PyQt6.QtCore import Qt, QSize, QPoint, QEvent

from viewer.image_canvas import ImageCanvas
from viewer.thumbnail_sidebar import ThumbnailSidebar, is_image_file


class MainWindow(QMainWindow):
    """The main image viewer window."""

    WINDOW_TITLE = "FOX Image Viewer"

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle(self.WINDOW_TITLE)
        self.resize(1200, 800)
        self.setMinimumSize(600, 400)

        # Frameless window — we provide our own title bar
        self.setWindowFlags(
            Qt.WindowType.FramelessWindowHint | Qt.WindowType.Window
        )
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground, False)

        # Drag state for custom title bar
        self._drag_pos: QPoint | None = None

        self._current_file: str | None = None
        self._current_pixmap: QPixmap | None = None

        # ── Widgets ──────────────────────────────────────────────
        self._canvas = ImageCanvas(self)
        self._sidebar = ThumbnailSidebar(self)

        # Central layout: sidebar | canvas
        splitter = QSplitter(Qt.Orientation.Horizontal, self)
        splitter.addWidget(self._sidebar)
        splitter.addWidget(self._canvas)
        splitter.setStretchFactor(0, 0)
        splitter.setStretchFactor(1, 1)
        splitter.setSizes([168, 1032])
        self.setCentralWidget(splitter)

        # Status bar
        self._status_label = QLabel("No image loaded")
        self._zoom_label = QLabel("")
        self._dim_label = QLabel("")
        status = QStatusBar(self)
        status.addWidget(self._status_label, 1)
        status.addPermanentWidget(self._dim_label)
        status.addPermanentWidget(self._zoom_label)
        self.setStatusBar(status)

        # ── Connect signals ──────────────────────────────────────
        self._sidebar.image_selected.connect(self._on_sidebar_select)
        self._canvas.zoom_changed.connect(self._on_zoom_changed)

        # ── Build menus & window controls ────────────────────────
        self._build_actions()
        self._build_menu()

        # Install event filter on menu bar for drag-to-move
        self.menuBar().installEventFilter(self)

        # ── Apply dark grey stylesheet ───────────────────────────
        self._apply_stylesheet()

    # ══════════════════════════════════════════════════════════════
    # Actions
    # ══════════════════════════════════════════════════════════════

    def _build_actions(self):
        # File actions
        self._act_open = QAction("&Open...", self)
        self._act_open.setShortcut(QKeySequence.StandardKey.Open)
        self._act_open.triggered.connect(self._on_open)

        self._act_open_dir = QAction("Open &Folder...", self)
        self._act_open_dir.setShortcut(QKeySequence("Ctrl+Shift+O"))
        self._act_open_dir.triggered.connect(self._on_open_dir)

        self._act_quit = QAction("&Quit", self)
        self._act_quit.setShortcut(QKeySequence.StandardKey.Quit)
        self._act_quit.triggered.connect(self.close)

        # View actions
        self._act_zoom_in = QAction("Zoom &In", self)
        self._act_zoom_in.setShortcut(QKeySequence("Ctrl+="))
        self._act_zoom_in.triggered.connect(self._canvas.zoom_in)

        self._act_zoom_out = QAction("Zoom &Out", self)
        self._act_zoom_out.setShortcut(QKeySequence("Ctrl+-"))
        self._act_zoom_out.triggered.connect(self._canvas.zoom_out)

        self._act_zoom_fit = QAction("&Fit to Window", self)
        self._act_zoom_fit.setShortcut(QKeySequence("Ctrl+0"))
        self._act_zoom_fit.triggered.connect(self._canvas.fit_to_view)

        self._act_zoom_orig = QAction("&Original Size (100%)", self)
        self._act_zoom_orig.setShortcut(QKeySequence("Ctrl+1"))
        self._act_zoom_orig.triggered.connect(self._canvas.zoom_original)

        self._act_rotate_cw = QAction("Rotate &Clockwise", self)
        self._act_rotate_cw.setShortcut(QKeySequence("R"))
        self._act_rotate_cw.triggered.connect(self._canvas.rotate_cw)

        self._act_rotate_ccw = QAction("Rotate Counter-Clock&wise", self)
        self._act_rotate_ccw.setShortcut(QKeySequence("Shift+R"))
        self._act_rotate_ccw.triggered.connect(self._canvas.rotate_ccw)

        self._act_fullscreen = QAction("&Fullscreen", self)
        self._act_fullscreen.setShortcut(QKeySequence("F11"))
        self._act_fullscreen.setCheckable(True)
        self._act_fullscreen.triggered.connect(self._toggle_fullscreen)

        # Navigation actions
        self._act_next = QAction("&Next Image", self)
        self._act_next.setShortcuts([QKeySequence("Right"), QKeySequence("Space")])
        self._act_next.triggered.connect(self._on_next)

        self._act_prev = QAction("&Previous Image", self)
        self._act_prev.setShortcuts([QKeySequence("Left"), QKeySequence("Backspace")])
        self._act_prev.triggered.connect(self._on_prev)

        # Sidebar toggle
        self._act_toggle_sidebar = QAction("&Sidebar", self)
        self._act_toggle_sidebar.setShortcut(QKeySequence("F9"))
        self._act_toggle_sidebar.setCheckable(True)
        self._act_toggle_sidebar.setChecked(True)
        self._act_toggle_sidebar.triggered.connect(self._toggle_sidebar)

        # Help
        self._act_about = QAction("&About", self)
        self._act_about.triggered.connect(self._show_about)

    def _build_menu(self):
        menu_bar = self.menuBar()

        file_menu = menu_bar.addMenu("&File")
        file_menu.addAction(self._act_open)
        file_menu.addAction(self._act_open_dir)
        file_menu.addSeparator()
        file_menu.addAction(self._act_quit)

        view_menu = menu_bar.addMenu("&View")
        view_menu.addAction(self._act_zoom_in)
        view_menu.addAction(self._act_zoom_out)
        view_menu.addAction(self._act_zoom_fit)
        view_menu.addAction(self._act_zoom_orig)
        view_menu.addSeparator()
        view_menu.addAction(self._act_rotate_cw)
        view_menu.addAction(self._act_rotate_ccw)
        view_menu.addSeparator()
        view_menu.addAction(self._act_fullscreen)
        view_menu.addAction(self._act_toggle_sidebar)

        nav_menu = menu_bar.addMenu("&Navigate")
        nav_menu.addAction(self._act_next)
        nav_menu.addAction(self._act_prev)

        help_menu = menu_bar.addMenu("&Help")
        help_menu.addAction(self._act_about)

        # ── Title label (centered in the remaining space) ────────
        spacer = QWidget()
        spacer.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        menu_bar.setCornerWidget(self._build_window_controls(), Qt.Corner.TopRightCorner)

    def _build_window_controls(self) -> QWidget:
        """Create minimize / maximize / close buttons for the title bar."""
        container = QWidget()
        layout = QHBoxLayout(container)
        layout.setContentsMargins(0, 0, 4, 0)
        layout.setSpacing(2)

        # Window title label
        self._title_label = QLabel(self.WINDOW_TITLE)
        self._title_label.setStyleSheet(
            "color: #cccccc; font-size: 14px; padding: 0 12px;"
        )
        layout.addWidget(self._title_label)

        btn_style_base = """
            QPushButton {{
                background-color: {bg};
                border: none;
                border-radius: 0px;
                min-width: 40px;
                max-width: 40px;
                min-height: 28px;
                max-height: 28px;
                font-size: 16px;
                color: #cccccc;
            }}
            QPushButton:hover {{
                background-color: {hover};
            }}
        """

        # Minimize
        btn_min = QPushButton("─")
        btn_min.setStyleSheet(btn_style_base.format(bg="transparent", hover="#444444"))
        btn_min.setToolTip("Minimize")
        btn_min.clicked.connect(self.showMinimized)
        layout.addWidget(btn_min)

        # Maximize / Restore
        self._btn_max = QPushButton("□")
        self._btn_max.setStyleSheet(btn_style_base.format(bg="transparent", hover="#444444"))
        self._btn_max.setToolTip("Maximize")
        self._btn_max.clicked.connect(self._toggle_maximize)
        layout.addWidget(self._btn_max)

        # Close
        btn_close = QPushButton("✕")
        btn_close.setStyleSheet(btn_style_base.format(bg="transparent", hover="#c42b1c"))
        btn_close.setToolTip("Close")
        btn_close.clicked.connect(self.close)
        layout.addWidget(btn_close)

        return container

    def _apply_stylesheet(self):
        """Apply a dark grey theme to the entire window."""
        self.setStyleSheet("""
            QMainWindow {
                background-color: #2b2b2b;
            }
            QMenuBar {
                background-color: #383838;
                color: #cccccc;
                border-bottom: 1px solid #4a4a4a;
                padding: 2px 0;
                font-size: 14px;
            }
            QMenuBar::item {
                padding: 6px 12px;
                border-radius: 4px;
            }
            QMenuBar::item:selected {
                background-color: #505050;
            }
            QMenu {
                background-color: #2b2b2b;
                color: #cccccc;
                border: 1px solid #3a3a3a;
                font-size: 14px;
            }
            QMenu::item:selected {
                background-color: #444444;
            }
            QMenu::separator {
                height: 1px;
                background-color: #3a3a3a;
                margin: 4px 8px;
            }
            QStatusBar {
                background-color: #2b2b2b;
                color: #999999;
                border-top: 1px solid #3a3a3a;
                font-size: 14px;
            }
            QSplitter::handle {
                background-color: #3a3a3a;
                width: 1px;
            }
        """)

    def _toggle_maximize(self):
        if self.isMaximized():
            self.showNormal()
            self._btn_max.setText("□")
            self._btn_max.setToolTip("Maximize")
        else:
            self.showMaximized()
            self._btn_max.setText("❐")
            self._btn_max.setToolTip("Restore")

    # ══════════════════════════════════════════════════════════════
    # Public API
    # ══════════════════════════════════════════════════════════════

    def open_image(self, filepath: str) -> None:
        """Open and display an image file, also loading its directory."""
        filepath = os.path.abspath(filepath)
        if not os.path.isfile(filepath):
            self._status_label.setText(f"File not found: {filepath}")
            return

        pixmap = QPixmap(filepath)
        if pixmap.isNull():
            self._status_label.setText(f"Cannot load: {filepath}")
            return

        self._current_file = filepath
        self._current_pixmap = pixmap
        self._canvas.load_pixmap(pixmap)

        # Update window title and status
        name = Path(filepath).name
        self.setWindowTitle(f"{name} — {self.WINDOW_TITLE}")
        self._title_label.setText(f"{name} — {self.WINDOW_TITLE}")
        self._status_label.setText(filepath)
        self._dim_label.setText(f"{pixmap.width()} × {pixmap.height()}")

        # Load the containing directory into the sidebar
        directory = str(Path(filepath).parent)
        self._sidebar.load_directory(directory, select_file=filepath)

    # ══════════════════════════════════════════════════════════════
    # Slots
    # ══════════════════════════════════════════════════════════════

    def _on_open(self):
        formats = " ".join(f"*.{fmt.data().decode()}" for fmt in QImageReader.supportedImageFormats())
        path, _ = QFileDialog.getOpenFileName(
            self,
            "Open Image",
            str(Path.home()),
            f"Images ({formats});;All Files (*)",
        )
        if path:
            self.open_image(path)

    def _on_open_dir(self):
        directory = QFileDialog.getExistingDirectory(
            self,
            "Open Folder",
            str(Path.home()),
        )
        if directory:
            self._sidebar.load_directory(directory)
            files = self._sidebar.get_file_list()
            if files:
                self.open_image(files[0])

    def _on_sidebar_select(self, filepath: str):
        if filepath != self._current_file:
            self.open_image(filepath)

    def _on_next(self):
        result = self._sidebar.next_image()
        # Signal handles the rest via _on_sidebar_select

    def _on_prev(self):
        result = self._sidebar.prev_image()
        # Signal handles the rest via _on_sidebar_select

    def _on_zoom_changed(self, percent: float):
        self._zoom_label.setText(f"{percent:.0f}%")

    def _toggle_fullscreen(self, checked: bool):
        if checked:
            self.showFullScreen()
        else:
            self.showNormal()
            self._btn_max.setText("□")
            self._btn_max.setToolTip("Maximize")

    def _toggle_sidebar(self, checked: bool):
        self._sidebar.setVisible(checked)

    def _show_about(self):
        QMessageBox.about(
            self,
            "About FOX Image Viewer",
            "<h2>FOX Image Viewer</h2>"
            "<p>A simple, fast image viewer for KDE Plasma.</p>"
            "<p><b>Features:</b></p>"
            "<ul>"
            "<li>Zoom with scroll wheel</li>"
            "<li>Pan with middle-click or Ctrl+drag</li>"
            "<li>Navigate with arrow keys</li>"
            "<li>Rotate with R / Shift+R</li>"
            "<li>Fullscreen with F11</li>"
            "<li>Double-click to fit to view</li>"
            "</ul>"
            "<p>Part of the <b>FOX-DE</b> project.</p>",
        )

    # ══════════════════════════════════════════════════════════════
    # Events
    # ══════════════════════════════════════════════════════════════

    def keyPressEvent(self, event):
        if event.key() == Qt.Key.Key_Escape:
            if self.isFullScreen():
                self._act_fullscreen.setChecked(False)
                self.showNormal()
                self._btn_max.setText("□")
                self._btn_max.setToolTip("Maximize")
                return
        super().keyPressEvent(event)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        # Re-fit image on window resize if we're in fit-to-view mode
        if self._canvas.has_image():
            self._canvas.fit_to_view()

    # ── Custom title bar dragging via event filter ────────────────

    def eventFilter(self, obj, event: QEvent) -> bool:
        """Intercept mouse events on the menu bar for window dragging."""
        if obj is self.menuBar():
            if event.type() == QEvent.Type.MouseButtonPress:
                me = event
                if me.button() == Qt.MouseButton.LeftButton:
                    # Only start drag if not clicking on an actual menu item
                    action = self.menuBar().actionAt(me.pos())
                    if action is None:
                        self._drag_pos = me.globalPosition().toPoint() - self.frameGeometry().topLeft()
                        return True

            elif event.type() == QEvent.Type.MouseMove:
                me = event
                if self._drag_pos is not None and me.buttons() & Qt.MouseButton.LeftButton:
                    if self.isMaximized():
                        self.showNormal()
                        self._btn_max.setText("□")
                        self._btn_max.setToolTip("Maximize")
                        self._drag_pos = QPoint(self.width() // 2, 15)
                    self.move(me.globalPosition().toPoint() - self._drag_pos)
                    return True

            elif event.type() == QEvent.Type.MouseButtonRelease:
                if self._drag_pos is not None:
                    self._drag_pos = None
                    return True

            elif event.type() == QEvent.Type.MouseButtonDblClick:
                me = event
                if me.button() == Qt.MouseButton.LeftButton:
                    action = self.menuBar().actionAt(me.pos())
                    if action is None:
                        self._toggle_maximize()
                        return True

        return super().eventFilter(obj, event)

    def dragEnterEvent(self, event):
        if event.mimeData().hasUrls():
            for url in event.mimeData().urls():
                if url.isLocalFile() and is_image_file(url.toLocalFile()):
                    event.acceptProposedAction()
                    return
        event.ignore()

    def dropEvent(self, event):
        for url in event.mimeData().urls():
            if url.isLocalFile():
                filepath = url.toLocalFile()
                if is_image_file(filepath):
                    self.open_image(filepath)
                    return
                elif os.path.isdir(filepath):
                    self._sidebar.load_directory(filepath)
                    files = self._sidebar.get_file_list()
                    if files:
                        self.open_image(files[0])
                    return
