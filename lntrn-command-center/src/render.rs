//! Panel chrome rendering.
//!
//! Phase 1: glassy panel chrome + open/close animation.
//! Phase 2.1: also draws the search input row.
//!
//! The animation is applied entirely in shader-space: the wl_surface
//! never resizes. We scale the panel rect around its own center and
//! modulate alpha by the same eased factor.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::app::{AppState, PanelRect, ANIM_SCALE_START, PANEL_CORNER_RADIUS};

/// Panel surface color — Fox Dark `bg` from lntrn-terminal/src/theme.rs:29
/// (#181818, rgb 24,24,24). Keeping the whole DE visually consistent.
///
/// IMPORTANT: the wgpu surface format is `Bgra8UnormSrgb`, so the GPU
/// applies sRGB gamma encoding at output. That means values written to
/// the framebuffer are interpreted as **linear**, not sRGB. We use
/// `Color::from_rgb8` which does the sRGB→linear conversion for us;
/// passing raw `Color::rgba(0.094, ...)` would treat 0.094 as linear and
/// display way too bright (about #5d5d5d on screen).
const SURFACE_BYTES: (u8, u8, u8) = (24, 24, 24);
/// Surface alpha when fully open. Fully opaque — matches the terminal.
/// Fallback panel opacity when no config has loaded. The actual value
/// per frame comes from [`crate::app::AppState::config::panel_opacity`].
const SURFACE_ALPHA_DEFAULT: f32 = 0.92;

/// Result of `draw_panel` — the (animated) panel rect and alpha, so
/// content layers can position themselves over the same region.
pub struct PanelDraw {
    pub rect: Rect,
    pub alpha: f32,
    pub scale_factor: f32,
}

/// One pending icon-render request. Drawing collects these as it walks
/// the layout; the render loop then turns each into a `TextureDraw` by
/// asking the `IconCache` for the actual texture (loading on miss).
///
/// We split it this way so launcher/search modules don't need to touch
/// the GPU directly — they just push owned `IconRequest`s into a Vec.
#[derive(Debug, Clone)]
pub struct IconRequest {
    pub app_id: String,
    pub icon_name: Option<String>,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub opacity: f32,
    /// Optional scissor `[x, y, w, h]` in physical pixels. Used by the
    /// scrollable grid so icons crossing the viewport edge get clipped
    /// instead of bleeding into the search bar / panel chrome.
    pub clip: Option<[f32; 4]>,
}

/// Draw the Command Center chrome at the current animation state.
///
/// Caller is responsible for clearing the surface to transparent before
/// calling this — we draw only the panel, not the fullscreen backdrop.
pub fn draw_panel(
    painter: &mut Painter,
    state: &AppState,
    surface_w: u32,
    scale: f32,
) -> Option<PanelDraw> {
    // Caller is responsible for `painter.clear()` at the start of the
    // frame. We just queue draws.
    let factor = state.anim_factor();
    if factor <= 0.0 {
        return None;
    }

    let alpha = factor;
    // Scale: ANIM_SCALE_START → 1.0 as factor goes 0 → 1.
    let s = ANIM_SCALE_START + (1.0 - ANIM_SCALE_START) * factor;

    let base = PanelRect::compute_with_dims(
        surface_w,
        scale,
        state.desired_panel_w_logical(),
        state.desired_panel_h_logical(),
    );

    // Scale the rect around its own center so it grows from the middle.
    let cx = base.x + base.w / 2.0;
    let cy = base.y + base.h / 2.0;
    let scaled_w = base.w * s;
    let scaled_h = base.h * s;
    let rect = Rect::new(
        cx - scaled_w / 2.0,
        cy - scaled_h / 2.0,
        scaled_w,
        scaled_h,
    );

    let radius = PANEL_CORNER_RADIUS * scale;

    // Drop shadow (pure black, soft, slightly offset down).
    let shadow_alpha = 0.35 * alpha;
    painter.shadow(
        rect,
        radius,
        24.0 * scale,
        Color::BLACK.with_alpha(shadow_alpha),
        0.0,
        4.0 * scale,
    );

    // Panel surface (matches Fox Dark terminal bg, sRGB-aware).
    let (sr, sg, sb) = SURFACE_BYTES;
    let opacity = state.config.panel_opacity.clamp(0.10, 1.0);
    let _ = SURFACE_ALPHA_DEFAULT; // silence the "unused" lint
    let surface = Color::from_rgb8(sr, sg, sb).with_alpha(opacity * alpha);
    painter.rect_filled(rect, radius, surface);

    // No border — keeps the panel reading as a single soft slab.

    // Scale the *content* by the same animation factor as the chrome,
    // so search/launcher/controls all grow from the panel center together.
    Some(PanelDraw {
        rect,
        alpha,
        scale_factor: scale * s,
    })
}

/// Draw all panel content. Called only when `draw_panel` returned `Some`.
/// Returns a Vec of icon requests that the render loop turns into
/// `TextureDraw`s.
///
/// Layout: the controls row + underline always sits at the top. Below
/// it, exactly one of two views fills the rest of the panel:
/// - `PanelMode::Launcher` → search input + pinned/results
/// - `PanelMode::Control(id)` → that control's full-content view
pub fn draw_content(
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    state: &AppState,
    panel: &PanelDraw,
    surface_w: u32,
    surface_h: u32,
) -> Vec<IconRequest> {
    let mut icons = Vec::new();

    // Highlight the tile whose view is currently showing.
    let selected_tile = match state.mode {
        crate::app::PanelMode::Control(id) => Some(id),
        crate::app::PanelMode::Launcher => None,
    };

    // Compute the side-to-side slide. When active we render BOTH the
    // outgoing and incoming view simultaneously, each at its own x
    // offset — they cross-fade like a carousel instead of one finishing
    // before the next starts.
    let slide = state.view_slide();
    let sliding = slide.is_some();
    let original_panel_rect = panel.rect;
    let view_pairs: Vec<(crate::app::PanelView, f32)> = match slide {
        Some(s) => vec![(s.from, s.from_offset), (s.to, s.to_offset)],
        None => vec![(state.panel_view, 0.0)],
    };
    let shift = |off_frac: f32| -> lntrn_render::Rect {
        lntrn_render::Rect::new(
            panel.rect.x + off_frac * panel.rect.w,
            panel.rect.y,
            panel.rect.w,
            panel.rect.h,
        )
    };
    let push_panel_clip = |painter: &mut Painter, text: &mut TextRenderer, mono_text: &mut TextRenderer| {
        painter.push_clip(original_panel_rect);
        let r = [
            original_panel_rect.x,
            original_panel_rect.y,
            original_panel_rect.w,
            original_panel_rect.h,
        ];
        text.push_clip(r);
        mono_text.push_clip(r);
    };
    let pop_panel_clip = |painter: &mut Painter, text: &mut TextRenderer, mono_text: &mut TextRenderer| {
        painter.pop_clip();
        text.pop_clip();
        mono_text.pop_clip();
    };

    // 1. Controls row + underline. When a slide is active each view
    //    is drawn in its own shifted rect; they glide past one another.
    if sliding {
        push_panel_clip(painter, text, mono_text);
    }
    for (view, off_frac) in &view_pairs {
        crate::controls::draw_row(
            painter,
            text,
            &state.controls,
            selected_tile,
            state.hovered_control_tile,
            shift(*off_frac),
            panel.scale_factor,
            panel.alpha,
            surface_w,
            surface_h,
            &mut icons,
            *view,
        );
    }
    if sliding {
        pop_panel_clip(painter, text, mono_text);
    }

    // Visibility of "Default-view chrome" (power column, mini-dock,
    // dock preview). When the user is transitioning into/out of the
    // Default view we fade these alongside the slide so they don't
    // pop in and out.
    let default_visibility: f32 = view_pairs
        .iter()
        .filter(|(v, _)| *v == crate::app::PanelView::Default)
        .map(|(_, off)| (1.0 - off.abs()).clamp(0.0, 1.0))
        .next()
        .unwrap_or(0.0);

    // 1b. Power column floating to the right of the panel. Fades with
    //     the collapse animation AND with the view slide so it never
    //     pops on/off.
    let collapse_p = state.collapse_progress();
    let chrome_alpha_mult = (1.0 - collapse_p).clamp(0.0, 1.0);
    let power_alpha_mult = chrome_alpha_mult * default_visibility;
    if power_alpha_mult > 0.005 {
        crate::power::draw(
            painter,
            &mut icons,
            panel.rect,
            panel.scale_factor,
            panel.alpha * power_alpha_mult,
            state.power_hover,
        );
    }

    // 1d. Mini-dock of pinned apps under the panel. Fades with collapse
    //     progress AND with `default_visibility` so it cross-fades
    //     during a view slide instead of popping. Skipped entirely
    //     when the user has disabled the collapsed-dock in settings,
    //     OR when any full-page overlay is open — those replace the
    //     panel body and the hover-preview tile would float on top.
    {
        let any_overlay_open = state.settings_open
            || state.emojis.open
            || state.clipboard.open
            || state.notes.open
            || state.usage.open;
        let dock_alpha_mult = if any_overlay_open {
            0.0
        } else {
            collapse_p.clamp(0.0, 1.0) * default_visibility
        };
        if dock_alpha_mult > 0.005 && state.config.show_dock_collapsed {
            let pinned = state.launcher.pinned_entries(&state.apps);
            let layout = crate::mini_dock::compute_layout(
                panel.rect,
                surface_h as f32,
                panel.scale_factor,
                &pinned,
                &state.toplevels,
                &state.apps,
                Some(state.cursor_phys),
            );
            if let Some(layout) = layout {
                crate::mini_dock::draw(
                    painter,
                    &mut icons,
                    &layout,
                    &state.toplevels,
                    panel.alpha * dock_alpha_mult,
                    state.mini_dock_hover,
                );

                // Hover preview tile above the icon when the hovered app
                // has at least one open window. Suppressed in alternate
                // top-level views (Music, Assistant) so the preview
                // doesn't collide with their bodies.
                let preview_hover = if state.panel_view == crate::app::PanelView::Default {
                    state.mini_dock_hover
                } else {
                    None
                };
                if let Some(hover_idx) = preview_hover {
                    if let Some(entry) = layout.entries.get(hover_idx) {
                        let windows = crate::mini_dock::windows_for_app(
                            &state.toplevels,
                            &entry.app_id,
                        );
                        if !windows.is_empty() {
                            let tiles = crate::mini_dock::preview_tile_rects(
                                &layout,
                                panel.rect,
                                hover_idx,
                                windows.len(),
                            );
                            if !tiles.is_empty() {
                                crate::mini_dock::draw_preview(
                                    painter,
                                    text,
                                    &mut icons,
                                    entry,
                                    &windows,
                                    &tiles,
                                    panel.scale_factor,
                                    panel.alpha * dock_alpha_mult,
                                    surface_w,
                                    surface_h,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // 1e. View-switcher arrows next to the controls row. Anchored at
    //     the top so they remain reachable while the panel is
    //     collapsed — they don't fade with the body content.
    crate::view_arrows::draw(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        state.view_arrow_hover,
    );

    // 1f. Desktop Settings button — floats under the left arrow, same column.
    crate::desktop_settings::draw_button(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        state.desktop_button_hover,
        state.desktop_settings_open,
    );

    // 1g. View dots + Home button in the strip above the panel.
    crate::view_indicator::draw_dots(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        state.panel_view,
    );
    crate::view_indicator::draw_home(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        state.home_hover,
    );
    // Grow button reflects the state it would toggle: collapsed → bar's
    // size step, expanded → window's size step. Show "shrink" arrows
    // (inward) only when the next click wraps back to small — i.e. when
    // already at the largest step.
    let size_idx = if state.collapsed {
        state.bar_size_idx
    } else {
        state.panel_size_idx
    };
    let grown_visual = size_idx >= crate::app::GROW_MAX_STEP;
    crate::view_indicator::draw_grow(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        grown_visual,
        state.grow_hover,
    );
    crate::view_indicator::draw_gear(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        state.gear_hover,
        state.settings_open,
    );
    crate::view_indicator::draw_emoji(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        false,
        state.emoji_hover,
    );
    crate::view_indicator::draw_clipboard(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        false,
        state.clipboard_hover,
    );
    crate::view_indicator::draw_notes(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        false,
        state.notes_hover,
    );
    // Claude-usage button — floats under the right arrow (mirrors
    // desktop button under the left arrow).
    crate::usage_button::draw(
        painter,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        state.usage_hover,
        state.usage.open,
        &mut icons,
    );

    // 1f + 1c. Per-view chevron and (Terminal-only) cursor-row mirror.
    // Same iteration as the tile row so they slide with it.
    if sliding {
        push_panel_clip(painter, text, mono_text);
    }
    for (view, off_frac) in &view_pairs {
        let r = shift(*off_frac);
        // Terminal input strip.
        if *view == crate::app::PanelView::Terminal {
            if let Some(chev) = state.controls.tile_layout(
                crate::controls::TileId::Collapse,
                r,
                panel.scale_factor,
                *view,
            ) {
                let gap = 12.0 * panel.scale_factor;
                let pad = crate::controls::ROW_HORIZONTAL_PAD * panel.scale_factor;
                let x = r.x + pad;
                let right_edge = chev.x - gap;
                let w = (right_edge - x).max(0.0);
                let h = (44.0 * panel.scale_factor).min(chev.h - 4.0 * panel.scale_factor);
                let y = chev.y + (chev.h - h) / 2.0;
                if w > 0.0 {
                    crate::terminal::draw_input_strip(
                        painter,
                        text,
                        mono_text,
                        &state.terminal,
                        lntrn_render::Rect::new(x, y, w, h),
                        panel.scale_factor,
                        panel.alpha,
                        surface_w,
                        surface_h,
                    );
                }
            }
        }
        // Files toolbar — back/home, breadcrumb/search pathbar, magnifier, eye.
        // Lives in the top-most controls row so it's reachable even while collapsed.
        if *view == crate::app::PanelView::Files {
            if let Some(chev) = state.controls.tile_layout(
                crate::controls::TileId::Collapse,
                r,
                panel.scale_factor,
                *view,
            ) {
                let gap = 12.0 * panel.scale_factor;
                let pad = crate::controls::ROW_HORIZONTAL_PAD * panel.scale_factor;
                let x = r.x + pad;
                let right_edge = chev.x - gap;
                let w = (right_edge - x).max(0.0);
                let h = chev.h - 4.0 * panel.scale_factor;
                let y = chev.y + 2.0 * panel.scale_factor;
                if w > 0.0 {
                    crate::files::draw_controls_strip(
                        painter,
                        text,
                        &state.files,
                        lntrn_render::Rect::new(x, y, w, h),
                        panel.scale_factor,
                        state.config.text_size,
                        panel.alpha,
                        surface_w,
                        surface_h,
                    );
                }
            }
        }
        // Collapse chevron.
        if let Some(layout) = state.controls.tile_layout(
            crate::controls::TileId::Collapse,
            r,
            panel.scale_factor,
            *view,
        ) {
            crate::controls::collapse::draw_inline(
                painter,
                text,
                state.collapsed,
                &layout,
                panel.scale_factor,
                panel.alpha,
                state.hovered_control_tile == Some(crate::controls::TileId::Collapse),
            );
        }
    }
    if sliding {
        pop_panel_clip(painter, text, mono_text);
    }

    // 2. Body of the panel, based on mode. Faded out during collapse
    //    so the content gracefully disappears as the panel shrinks
    //    (and back in on expand).
    if chrome_alpha_mult < 0.005 {
        return icons;
    }
    let body_alpha_base = panel.alpha * chrome_alpha_mult;
    let icons_before_body = icons.len();

    // Settings overlay short-circuits the per-view body draw — it's a
    // dedicated page that doesn't participate in the view slide.
    if state.settings_open {
        let top_y = crate::controls::content_top_y(panel.rect, panel.scale_factor);
        crate::settings::draw(
            painter,
            text,
            &state.config,
            panel.rect,
            top_y,
            panel.scale_factor,
            body_alpha_base,
            surface_w,
            surface_h,
        );
        return icons;
    }
    // Emojis overlay short-circuits the per-view body draw too.
    if state.emojis.open {
        let top_y = crate::controls::content_top_y(panel.rect, panel.scale_factor);
        crate::emojis::render::draw(
            painter,
            text,
            &mut icons,
            &state.emojis,
            panel.rect,
            top_y,
            panel.scale_factor,
            state.config.text_size,
            body_alpha_base,
            surface_w,
            surface_h,
        );
        return icons;
    }
    // Clipboard History overlay short-circuits the per-view body draw.
    if state.clipboard.open {
        let top_y = crate::controls::content_top_y(panel.rect, panel.scale_factor);
        crate::clipboard::render::draw(
            painter,
            text,
            &mut icons,
            &state.clipboard,
            panel.rect,
            top_y,
            panel.scale_factor,
            state.config.text_size,
            body_alpha_base,
            surface_w,
            surface_h,
        );
        return icons;
    }
    // Quick Notes overlay short-circuits the per-view body draw.
    if state.notes.open {
        let top_y = crate::controls::content_top_y(panel.rect, panel.scale_factor);
        crate::notes::render::draw(
            painter,
            text,
            &state.notes,
            panel.rect,
            top_y,
            panel.scale_factor,
            state.config.text_size,
            body_alpha_base,
            surface_w,
            surface_h,
        );
        return icons;
    }
    // Claude-usage overlay.
    if state.usage.open {
        let top_y = crate::controls::content_top_y(panel.rect, panel.scale_factor);
        crate::usage::view::draw(
            painter,
            text,
            &state.usage.stats,
            panel.rect,
            top_y,
            panel.scale_factor,
            state.config.text_size,
            body_alpha_base,
            surface_w,
            surface_h,
        );
        return icons;
    }

    if sliding {
        push_panel_clip(painter, text, mono_text);
    }

    // Per-view body draw. When sliding, this loop runs twice: once for
    // the outgoing view at its current offset and once for the
    // incoming view at its current offset. The clip + icon clamping
    // around the loop keeps both bodies confined to the panel rect.
    for (view, off_frac) in &view_pairs {
        let r = shift(*off_frac);
        let alpha = body_alpha_base;

        if *view == crate::app::PanelView::Default {
            match state.mode {
                crate::app::PanelMode::Launcher => {
                    crate::search::draw_input(
                        painter,
                        text,
                        &state.search,
                        r,
                        panel.scale_factor,
                        alpha,
                        surface_w,
                        surface_h,
                        state.waffle_hover,
                    );

                    let selected_pin = match state.selection {
                        crate::app::Selection::Pin(i) => Some(i),
                        _ => None,
                    };
                    let selected_result = match state.selection {
                        crate::app::Selection::Result(i) => Some(i),
                        _ => None,
                    };

                    if state.search.input.is_empty() && !state.search.all_apps_mode {
                        let top_y = crate::search::content_top_y(r, panel.scale_factor);
                        let pins_bottom = crate::launcher::draw(
                            painter,
                            text,
                            &mut icons,
                            &state.launcher,
                            &state.apps,
                            selected_pin,
                            r,
                            top_y,
                            panel.scale_factor,
                            alpha,
                            surface_w,
                            surface_h,
                        );
                        crate::launcher::open::draw(
                            painter,
                            text,
                            &mut icons,
                            &state.toplevels,
                            &state.apps,
                            None,
                            r,
                            pins_bottom,
                            panel.scale_factor,
                            alpha,
                            surface_w,
                            surface_h,
                        );
                        if let Some(drag) = state.pin_drag.as_ref() {
                            let row_top = crate::launcher::pins_row_top_y(top_y, panel.scale_factor);
                            crate::launcher::draw_pin_drag_overlay(
                                painter,
                                &mut icons,
                                &state.launcher,
                                &state.apps,
                                drag,
                                r,
                                row_top,
                                panel.scale_factor,
                                alpha,
                            );
                        }
                    } else {
                        crate::search::draw_results(
                            painter,
                            text,
                            &mut icons,
                            &state.search,
                            &state.apps,
                            selected_result,
                            r,
                            panel.scale_factor,
                            alpha,
                            surface_w,
                            surface_h,
                        );
                    }
                }
                crate::app::PanelMode::Control(tile_id) => {
                    crate::controls::draw_view(
                        painter,
                        text,
                        &state.controls,
                        tile_id,
                        r,
                        panel.scale_factor,
                        alpha,
                        state.config.text_size,
                        surface_w,
                        surface_h,
                    );
                }
            }
        } else if *view == crate::app::PanelView::Terminal {
            let top_y = crate::controls::content_top_y(r, panel.scale_factor);
            crate::terminal::draw(
                painter,
                text,
                mono_text,
                &state.terminal,
                r,
                top_y,
                panel.scale_factor,
                alpha,
                surface_w,
                surface_h,
                state.config.text_size,
            );
        } else if *view == crate::app::PanelView::Files {
            let top_y = crate::controls::content_top_y(r, panel.scale_factor);
            crate::files::draw(
                painter,
                text,
                &mut icons,
                &state.files,
                r,
                top_y,
                panel.scale_factor,
                state.config.text_size,
                alpha,
                surface_w,
                surface_h,
            );
        }
        // Default + Terminal each have their own body renderers above.
        // No catch-all branch remains now that Assistant + MusicPlayer
        // are gone.
    }

    if sliding {
        pop_panel_clip(painter, text, mono_text);
        clamp_body_icon_clips(&mut icons, icons_before_body, original_panel_rect);
    }

    // 3. Right-click context menu — drawn on layer 1 so it sits above
    //    grid tiles, labels, and any other panel content. Clamped to
    //    the full surface (not the panel) so dock right-click menus,
    //    which anchor outside the panel rect, can appear at the cursor.
    if let Some(menu) = &state.context_menu {
        painter.set_layer(1);
        text.set_layer(1);
        let surface_bounds = lntrn_render::Rect::new(0.0, 0.0, surface_w as f32, surface_h as f32);
        crate::launcher::context_menu::draw(
            painter,
            text,
            menu,
            surface_bounds,
            panel.scale_factor,
            surface_w,
            surface_h,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    // 4. Calendar event context menu — same overlay layer so it sits
    //    above the day-detail panel.
    if let Some(menu) = &state.controls.clock.event_menu {
        painter.set_layer(1);
        text.set_layer(1);
        crate::controls::clock::draw_event_menu(
            painter,
            text,
            menu,
            panel.rect,
            panel.scale_factor,
            surface_w,
            surface_h,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    // 5. Power confirm modal — overlay so it sits above everything.
    if let Some(action) = state.power_confirm {
        painter.set_layer(1);
        text.set_layer(1);
        crate::power::draw_confirm(
            painter,
            text,
            &mut icons,
            action,
            surface_w,
            surface_h,
            panel.scale_factor,
            panel.alpha,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    // 6. Desktop Settings popover — floats above the strip button.
    if state.desktop_settings_open {
        painter.set_layer(1);
        text.set_layer(1);
        crate::desktop_settings::draw_popover(
            painter,
            text,
            panel.rect,
            panel.scale_factor,
            panel.alpha,
            &state.desktop_widgets,
            state.desktop_settings_hover,
            surface_w,
            surface_h,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    icons
}

/// Clamp the clip rect on each icon request in `icons[from..]` to the
/// given panel rect. Textures don't pick up the painter's clip stack,
/// so without this they happily render past the panel edge during the
/// view-switch slide.
fn clamp_body_icon_clips(icons: &mut [IconRequest], from: usize, panel: Rect) {
    if from >= icons.len() {
        return;
    }
    let clip = [panel.x, panel.y, panel.w, panel.h];
    for req in &mut icons[from..] {
        match req.clip {
            None => req.clip = Some(clip),
            Some(existing) => {
                // Intersect existing clip with the panel rect so
                // dock/preview clips stay tighter than the panel.
                let x = existing[0].max(panel.x);
                let y = existing[1].max(panel.y);
                let right = (existing[0] + existing[2]).min(panel.x + panel.w);
                let bottom = (existing[1] + existing[3]).min(panel.y + panel.h);
                req.clip = Some([x, y, (right - x).max(0.0), (bottom - y).max(0.0)]);
            }
        }
    }
}
