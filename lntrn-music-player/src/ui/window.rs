use eframe::egui;

const RESIZE_EDGE: f32 = 6.0;

// ── Window resize handles ────────────────────────────────────────────────────

pub fn handle_resize_edges(ctx: &egui::Context) {
    let screen = ctx.content_rect();

    let edges = [
        (
            egui::Rect::from_min_max(
                egui::pos2(screen.left(), screen.top()),
                egui::pos2(screen.right(), screen.top() + RESIZE_EDGE),
            ),
            egui::CursorIcon::ResizeNorth,
            egui::viewport::ResizeDirection::North,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(screen.left(), screen.bottom() - RESIZE_EDGE),
                egui::pos2(screen.right(), screen.bottom()),
            ),
            egui::CursorIcon::ResizeSouth,
            egui::viewport::ResizeDirection::South,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(screen.left(), screen.top()),
                egui::pos2(screen.left() + RESIZE_EDGE, screen.bottom()),
            ),
            egui::CursorIcon::ResizeWest,
            egui::viewport::ResizeDirection::West,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(screen.right() - RESIZE_EDGE, screen.top()),
                egui::pos2(screen.right(), screen.bottom()),
            ),
            egui::CursorIcon::ResizeEast,
            egui::viewport::ResizeDirection::East,
        ),
    ];

    let corners = [
        (
            egui::Rect::from_min_max(
                screen.left_top(),
                egui::pos2(
                    screen.left() + RESIZE_EDGE * 2.0,
                    screen.top() + RESIZE_EDGE * 2.0,
                ),
            ),
            egui::CursorIcon::ResizeNorthWest,
            egui::viewport::ResizeDirection::NorthWest,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(screen.right() - RESIZE_EDGE * 2.0, screen.top()),
                egui::pos2(screen.right(), screen.top() + RESIZE_EDGE * 2.0),
            ),
            egui::CursorIcon::ResizeNorthEast,
            egui::viewport::ResizeDirection::NorthEast,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(screen.left(), screen.bottom() - RESIZE_EDGE * 2.0),
                egui::pos2(
                    screen.left() + RESIZE_EDGE * 2.0,
                    screen.bottom(),
                ),
            ),
            egui::CursorIcon::ResizeSouthWest,
            egui::viewport::ResizeDirection::SouthWest,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(
                    screen.right() - RESIZE_EDGE * 2.0,
                    screen.bottom() - RESIZE_EDGE * 2.0,
                ),
                screen.right_bottom(),
            ),
            egui::CursorIcon::ResizeSouthEast,
            egui::viewport::ResizeDirection::SouthEast,
        ),
    ];

    for (rect, cursor, direction) in corners.iter().chain(edges.iter()) {
        let response = ctx.input(|i| {
            i.pointer
                .hover_pos()
                .map(|pos| rect.contains(pos))
                .unwrap_or(false)
        });

        if response {
            ctx.set_cursor_icon(*cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(*direction));
            }
        }
    }
}
