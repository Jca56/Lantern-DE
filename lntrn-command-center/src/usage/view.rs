//! Render the Claude-usage overlay page.
//!
//! Layout (top → bottom):
//!   0. Live limit bars: current 5-hour + weekly window utilization
//!      (same data Claude Code's `/usage` shows), color-coded.
//!   1. Three hero stat cards stacked side-by-side under the toolbar:
//!      `$ saved`, `Total tokens`, `Total turns`.
//!   2. Time-window rows: Today / This week / This month / All time.
//!   3. Per-project breakdown (top 8 by cost).
//!   4. Per-model split.
//!   5. Daily sparkline of the trailing 30 days.
//!   6. Misc footer: cache-hit ratio, web tools used, session count,
//!      date span.
//!
//! All text sizes scale off the user's `text_size` config so the panel
//! reflows correctly when the user bumps font sizes in settings.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::limits::{RateLimits, RateWindow};
use super::model_status::{ModelHealth, ModelStatus, MODEL_LABEL};
use super::pricing::short_name;
use super::stats::{DayBucket, UsageStats};

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a); // gold
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const MUTED_RGB: (u8, u8, u8) = (0xb8, 0xb0, 0xa6);

const PAD: f32 = 18.0;

fn accent(a: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(a)
}
fn white(a: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(a)
}
fn muted(a: f32) -> Color {
    Color::from_rgb8(MUTED_RGB.0, MUTED_RGB.1, MUTED_RGB.2).with_alpha(a)
}

/// Best-effort UTC "today" string in `YYYY-MM-DD`. Transcript timestamps
/// are UTC ISO-8601 so this matches the by-day bucket keys.
fn today_utc() -> String {
    cutoff_offset(0)
}
fn week_cutoff() -> String {
    cutoff_offset(7)
}
fn month_cutoff() -> String {
    cutoff_offset(30)
}
fn cutoff_offset(days_ago: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day_secs: i64 = 86_400;
    let days = secs as i64 / day_secs - days_ago;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    // howardhinnant.github.io/date_algorithms.html, `civil_from_days`.
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yfix = if m <= 2 { y + 1 } else { y };
    (yfix as i32, m as u32, d as u32)
}

fn fmt_dollars(v: f64) -> String {
    // Always two decimals so the column reads as a clean money table:
    // "$1.25", "$312.50", "$1.25k", "$17.61k", "$1.25M". The unit
    // suffix (none / k / M) is the ONLY thing that changes scale, so
    // $1.25 always means $1.25 and $1.25k always means $1,250.
    if v >= 1_000_000.0 {
        format!("${:.2}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("${:.2}k", v / 1_000.0)
    } else {
        format!("${:.2}", v)
    }
}

fn fmt_count(v: u64) -> String {
    // Same idea — keep a 2-decimal suffix-or-nothing rule so the
    // tokens column reads as one consistent format.
    if v >= 1_000_000_000 {
        format!("{:.2}B", v as f64 / 1_000_000_000.0)
    } else if v >= 1_000_000 {
        format!("{:.2}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.2}k", v as f64 / 1_000.0)
    } else {
        v.to_string()
    }
}

fn fmt_int(v: u64) -> String {
    let s = v.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        let from_right = bytes.len() - i;
        if i > 0 && from_right % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    stats: &UsageStats,
    limits: &RateLimits,
    model_status: &ModelStatus,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = PAD * scale;
    let panel_bottom = panel.y + panel.h;
    let content_x = panel.x + pad;
    let content_w = panel.w - pad * 2.0;
    let mut y = top_y + pad * 0.5;

    // Font budget — everything else is a ratio of `body`, which itself
    // tracks the user's text-size setting. Bumping text_size scales the
    // entire page proportionally.
    let body = (text_size * scale).max(14.0 * scale);
    let header = body * 1.10;
    let stat = body * 0.95;
    let chip = body * 0.85;
    let hero_label = body * 0.95;
    // The big hero number is hand-scaled per card width so it can still
    // dominate the layout when the user picks a small text-size.

    // 0a. Fable 5 availability — a compact "can I use it?" status row at
    //     the very top. Driven by the model_status poller (live probe).
    {
        let row_h = body * 1.7;
        draw_model_status_row(
            painter, text, content_x, y, content_w, row_h, scale, body, stat, chip,
            alpha, model_status, surface_w, surface_h,
        );
        y += row_h + pad * 0.6;
    }

    // 0. Live limit bars — current 5-hour + weekly window utilization.
    //    Hidden until the limits poller delivers its first snapshot
    //    (no token / offline → the page simply looks like it used to).
    let windows: [(&str, Option<&RateWindow>); 2] = [
        ("5-hour limit", limits.five_hour.as_ref()),
        ("Weekly limit", limits.seven_day.as_ref()),
    ];
    if windows.iter().any(|(_, w)| w.is_some()) {
        let row_h = body * 1.85;
        let mut drawn = 0u32;
        for (label, win) in windows.iter() {
            let Some(win) = win else { continue };
            let ry = y + drawn as f32 * row_h;
            draw_limit_row(
                painter, text, content_x, ry, content_w, row_h, scale,
                body, stat, chip, alpha, label, win, surface_w, surface_h,
            );
            drawn += 1;
        }
        y += drawn as f32 * row_h + pad * 0.7;
    }

    // 1. Hero strip — three big stat cards.
    let card_gap = 14.0 * scale;
    let card_h = (body * 4.0).clamp(78.0 * scale, 116.0 * scale);
    let card_w = (content_w - card_gap * 2.0) / 3.0;
    let cards: [(String, &str, bool); 3] = [
        (fmt_dollars(stats.cost_usd), "Saved vs API", true),
        (fmt_count(stats.total_tokens()), "Total tokens", false),
        (fmt_int(stats.turns), "Assistant turns", false),
    ];
    for (i, (big, label, headline)) in cards.iter().enumerate() {
        let x = content_x + (card_w + card_gap) * i as f32;
        let r = Rect::new(x, y, card_w, card_h);
        draw_card(
            painter,
            text,
            r,
            big,
            label,
            *headline,
            scale,
            body,
            hero_label,
            alpha,
            surface_w,
            surface_h,
        );
    }
    y += card_h + pad;

    // 2. Time-window rows: Today / Week / Month / All time. Three
    //    value columns ("Saved" / "Tokens" / "Turns") with shared
    //    right-edge anchors so each column lines up vertically. The
    //    project + model sections below reuse the SAME `saved_right`
    //    and `tokens_right` anchors so all three sections share an
    //    invisible grid.
    let today = today_utc();
    let (today_turns, today_tok, today_cost) = stats.since(today.as_str());
    let (week_turns, week_tok, week_cost) = stats.since(week_cutoff().as_str());
    let (month_turns, month_tok, month_cost) = stats.since(month_cutoff().as_str());
    let rows: [(&str, u64, u64, f64); 4] = [
        ("Today", today_turns, today_tok, today_cost),
        ("This week", week_turns, week_tok, week_cost),
        ("This month", month_turns, month_tok, month_cost),
        ("All time", stats.turns, stats.total_tokens(), stats.cost_usd),
    ];

    // Shared column-grid anchors used by every numeric row on the page.
    let turns_right = content_x + content_w;
    let tokens_right = turns_right - content_w * 0.18;
    let saved_right = tokens_right - content_w * 0.18;

    // Column headers — small muted labels right-aligned to the same
    // anchors so the user can read what each number means.
    let head = body * 0.78;
    let header_top = y;
    for (lbl, right_x) in [
        ("Saved", saved_right),
        ("Tokens", tokens_right),
        ("Turns", turns_right),
    ] {
        let w = text.measure_width(lbl, head);
        text.queue(
            lbl,
            head,
            right_x - w,
            header_top,
            muted(0.65 * alpha),
            w,
            surface_w,
            surface_h,
        );
    }
    y += head * 1.7;

    let row_h = body * 1.75;
    for (i, (label, turns, tok, cost)) in rows.iter().enumerate() {
        let ry = y + i as f32 * row_h;
        if ry + row_h > panel_bottom - pad {
            break;
        }
        // Subtle zebra under "All time" so the totals row reads as a
        // summary rather than equally-weighted.
        if i == 3 {
            painter.rect_filled(
                Rect::new(content_x - 4.0 * scale, ry, content_w + 8.0 * scale, row_h),
                10.0 * scale,
                white(0.04 * alpha),
            );
        }
        // Label.
        let l_top = ry + (row_h - body) * 0.5 - body * 0.15;
        text.queue(
            label,
            body,
            content_x + 6.0 * scale,
            l_top,
            white(0.92 * alpha),
            content_w * 0.40,
            surface_w,
            surface_h,
        );
        // Three value columns, right-aligned to shared anchors.
        let col_color = if i == 3 { accent(alpha) } else { muted(alpha) };
        let s_top = l_top + (body - stat) * 0.5;
        let values = [
            (fmt_dollars(*cost), saved_right),
            (fmt_count(*tok), tokens_right),
            (fmt_int(*turns), turns_right),
        ];
        for (v, right_x) in values.iter() {
            let vw = text.measure_width(v, stat);
            text.queue(v, stat, right_x - vw, s_top, col_color, vw, surface_w, surface_h);
        }
    }
    y += row_h * rows.len() as f32 + pad;

    // 3. Per-project breakdown.
    if y + body * 6.0 < panel_bottom - pad && !stats.by_project.is_empty() {
        text.queue(
            "Top projects",
            header,
            content_x,
            y,
            muted(alpha),
            content_w,
            surface_w,
            surface_h,
        );
        y += header * 1.6;
        let project_max_cost = stats
            .by_project
            .first()
            .map(|p| p.cost_usd)
            .unwrap_or(1.0)
            .max(1.0);
        let bucket_row_h = body * 1.55;
        let mut drawn = 0u32;
        for p in stats.by_project.iter().take(8) {
            let ry = y + drawn as f32 * bucket_row_h;
            if ry + bucket_row_h > panel_bottom - pad {
                break;
            }
            draw_bucket_row(
                painter,
                text,
                content_x,
                ry,
                content_w,
                scale,
                body,
                stat,
                alpha,
                &p.label,
                p.cost_usd,
                p.tokens,
                project_max_cost,
                saved_right,
                tokens_right,
                surface_w,
                surface_h,
            );
            drawn += 1;
        }
        y += drawn as f32 * bucket_row_h + pad * 0.6;
    }

    // 4. Model split.
    if y + body * 3.0 < panel_bottom - pad && !stats.by_model.is_empty() {
        text.queue(
            "Model split",
            header,
            content_x,
            y,
            muted(alpha),
            content_w,
            surface_w,
            surface_h,
        );
        y += header * 1.6;
        let model_max_cost = stats
            .by_model
            .first()
            .map(|m| m.cost_usd)
            .unwrap_or(1.0)
            .max(1.0);
        let bucket_row_h = body * 1.55;
        let mut drawn = 0u32;
        for m in stats.by_model.iter().take(4) {
            let ry = y + drawn as f32 * bucket_row_h;
            if ry + bucket_row_h > panel_bottom - pad {
                break;
            }
            let short = short_name(&m.label);
            draw_bucket_row(
                painter,
                text,
                content_x,
                ry,
                content_w,
                scale,
                body,
                stat,
                alpha,
                &short,
                m.cost_usd,
                m.tokens,
                model_max_cost,
                saved_right,
                tokens_right,
                surface_w,
                surface_h,
            );
            drawn += 1;
        }
        y += drawn as f32 * bucket_row_h + pad * 0.6;
    }

    // 5. Daily sparkline (trailing 30 days).
    if y + body * 4.5 < panel_bottom - pad && !stats.by_day.is_empty() {
        text.queue(
            "Last 30 days",
            header,
            content_x,
            y,
            muted(alpha),
            content_w,
            surface_w,
            surface_h,
        );
        y += header * 1.6;
        let spark_h = (body * 3.0).max(56.0 * scale);
        let spark_rect = Rect::new(content_x, y, content_w, spark_h);
        draw_sparkline(painter, spark_rect, scale, alpha, &stats.by_day);
        y += spark_h + pad * 0.6;
    }

    // 6. Footer chips.
    if y + chip * 2.5 < panel_bottom - pad {
        let hit_pct = stats.cache_hit_ratio() * 100.0;
        let footer = format!(
            "{:.1}% cache hits   ·   {} sessions   ·   {} web searches   ·   {} web fetches",
            hit_pct,
            fmt_int(stats.sessions),
            fmt_int(stats.web_search_requests),
            fmt_int(stats.web_fetch_requests),
        );
        text.queue(
            &footer,
            chip,
            content_x,
            y,
            muted(0.80 * alpha),
            content_w,
            surface_w,
            surface_h,
        );
        y += chip * 1.5;
        if let (Some(first), Some(last)) = (&stats.first_ts, &stats.last_ts) {
            let f = first.get(..10).unwrap_or(first.as_str());
            let l = last.get(..10).unwrap_or(last.as_str());
            let span = format!("{}  →  {}", f, l);
            let s_font = chip * 0.92;
            text.queue(
                &span,
                s_font,
                content_x,
                y,
                muted(0.55 * alpha),
                content_w,
                surface_w,
                surface_h,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    painter: &mut Painter,
    text: &mut TextRenderer,
    r: Rect,
    big: &str,
    label: &str,
    headline: bool,
    scale: f32,
    body: f32,
    label_font: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let bg_alpha = if headline { 0.10 } else { 0.06 };
    painter.rect_filled(r, 16.0 * scale, white(bg_alpha * alpha));
    if headline {
        painter.rect_stroke_sdf(r, 16.0 * scale, 1.5 * scale, accent(0.45 * alpha));
    }
    let big_color = if headline { accent(alpha) } else { white(alpha) };

    // Carve the card into two stacked zones so the big number and the
    // label never overlap regardless of how aggressively the number
    // grew to fit the card width.
    //   - Top 62% of the card: the big number, vertically centered.
    //   - Bottom 38%: the label, vertically centered.
    let label_zone_h = r.h * 0.36;
    let big_zone_h = r.h - label_zone_h;

    // Pick the biggest font that still fits the big-zone height AND
    // the card width (with padding). Bounded by the zone height so it
    // never spills into the label zone.
    let avail_w = r.w - 16.0 * scale;
    let height_cap = big_zone_h * 0.78;
    let cap = if headline { body * 3.0 } else { body * 2.5 };
    let floor = body * 1.4;
    let mut big_font = cap.min(height_cap);
    for _ in 0..6 {
        let w = text.measure_width(big, big_font);
        if w <= avail_w {
            break;
        }
        big_font *= 0.85;
        if big_font < floor {
            big_font = floor;
            break;
        }
    }
    let big_w = text.measure_width(big, big_font);
    let big_x = r.x + (r.w - big_w) * 0.5;
    let big_y = r.y + (big_zone_h - big_font) * 0.5;
    text.queue(
        big,
        big_font,
        big_x,
        big_y,
        big_color,
        r.w,
        surface_w,
        surface_h,
    );

    let label_w = text.measure_width(label, label_font);
    let label_x = r.x + (r.w - label_w) * 0.5;
    let label_y = r.y + big_zone_h + (label_zone_h - label_font) * 0.5;
    text.queue(
        label,
        label_font,
        label_x,
        label_y,
        muted(alpha),
        r.w,
        surface_w,
        surface_h,
    );
}

/// Compact model-availability row:
///   `● Claude Fable 5 ················ Available · checked 2m ago`
/// The dot + status word share a traffic-light color (green up, red down,
/// grey while we wait for the first answer); the age chip is muted.
#[allow(clippy::too_many_arguments)]
fn draw_model_status_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    scale: f32,
    body: f32,
    stat: f32,
    chip: f32,
    alpha: f32,
    status: &ModelStatus,
    surface_w: u32,
    surface_h: u32,
) {
    let (dot, word) = match status.health {
        ModelHealth::Up => (Color::from_rgb8(120, 220, 120), "Available"),
        ModelHealth::Down => (Color::from_rgb8(255, 90, 90), "Unavailable"),
        ModelHealth::Unknown => (Color::from_rgb8(150, 150, 150), "Checking\u{2026}"),
    };

    // Status dot (a fully-rounded square reads as a circle).
    let dot_d = (body * 0.55).clamp(8.0 * scale, 14.0 * scale);
    let dot_y = y + (row_h - dot_d) * 0.5;
    painter.rect_filled(
        Rect::new(x, dot_y, dot_d, dot_d),
        dot_d / 2.0,
        dot.with_alpha(alpha),
    );

    // Model label.
    let l_top = y + (row_h - body) * 0.5 - body * 0.15;
    let label_x = x + dot_d + 10.0 * scale;
    text.queue(
        MODEL_LABEL,
        body,
        label_x,
        l_top,
        white(0.92 * alpha),
        w * 0.5,
        surface_w,
        surface_h,
    );

    // Right edge: status word (colored), with an optional muted "checked
    // Nm ago" freshness chip to its right.
    let right = x + w;
    let mut word_right = right;
    let age = status
        .checked_at
        .map(|t| {
            let now = now_epoch();
            if now >= t {
                fmt_ago(now - t)
            } else {
                String::new()
            }
        })
        .filter(|s| !s.is_empty());
    if let Some(age) = age {
        let age_text = format!("· {age}");
        let aw = text.measure_width(&age_text, chip);
        let a_top = y + (row_h - chip) * 0.5 - chip * 0.15;
        text.queue(
            &age_text,
            chip,
            right - aw,
            a_top,
            muted(0.6 * alpha),
            // Half-em slack so the exact-width bound can't clip the last glyph.
            aw + chip * 0.5,
            surface_w,
            surface_h,
        );
        word_right = right - aw - 10.0 * scale;
    }
    let ww = text.measure_width(word, stat);
    let s_top = y + (row_h - stat) * 0.5 - stat * 0.15;
    text.queue(
        word,
        stat,
        word_right - ww,
        s_top,
        dot.with_alpha(alpha),
        ww + stat * 0.5,
        surface_w,
        surface_h,
    );
}

/// Coarse "time since" for the freshness chip: `just now`, `Nm ago`, `Nh ago`.
fn fmt_ago(secs: u64) -> String {
    let mins = secs / 60;
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else {
        format!("{}h ago", mins / 60)
    }
}

/// One live rate-limit window row:
///   `label ── [████████░░░░░░░░░] resets in 2h 41m  41%`
/// Bar + percent share a traffic-light color so a glance tells you how
/// hot the window is. Both rows use fixed column anchors so the bars
/// line up vertically.
#[allow(clippy::too_many_arguments)]
fn draw_limit_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    scale: f32,
    body: f32,
    stat: f32,
    chip: f32,
    alpha: f32,
    label: &str,
    win: &RateWindow,
    surface_w: u32,
    surface_h: u32,
) {
    // Traffic-light thresholds matching the shell statusline: green
    // under 60%, orange under 85%, red past that.
    let col = if win.utilization >= 85.0 {
        Color::from_rgb8(255, 90, 90)
    } else if win.utilization >= 60.0 {
        Color::from_rgb8(255, 180, 80)
    } else {
        Color::from_rgb8(120, 220, 120)
    };

    // Fixed right-edge anchors shared by both rows.
    let pct_right = x + w;
    let reset_right = pct_right - stat * 3.4;
    let bar_right = reset_right - chip * 8.6 - 14.0 * scale;
    let bar_left = x + w * 0.22;

    // Label.
    let l_top = y + (row_h - body) * 0.5 - body * 0.15;
    text.queue(
        label,
        body,
        x,
        l_top,
        white(0.92 * alpha),
        bar_left - x - 10.0 * scale,
        surface_w,
        surface_h,
    );

    // Track + fill. Chunkier than the breakdown bars below — these are
    // the live numbers you open the page for.
    let bar_h = (body * 0.50).clamp(8.0 * scale, 15.0 * scale);
    let bar_y = y + (row_h - bar_h) * 0.5;
    let bar_w = (bar_right - bar_left).max(40.0 * scale);
    painter.rect_filled(
        Rect::new(bar_left, bar_y, bar_w, bar_h),
        bar_h / 2.0,
        white(0.08 * alpha),
    );
    let frac = (win.utilization / 100.0).clamp(0.0, 1.0);
    if frac > 0.001 {
        painter.rect_filled(
            Rect::new(bar_left, bar_y, (bar_w * frac).max(bar_h), bar_h),
            bar_h / 2.0,
            col.with_alpha(alpha),
        );
    }

    // Reset countdown, muted, right-aligned to its own anchor.
    if let Some(reset) = win.resets_at {
        let now = now_epoch();
        if reset > now {
            let t = format!("resets in {}", fmt_remaining(reset - now));
            let tw = text.measure_width(&t, chip);
            let t_top = y + (row_h - chip) * 0.5 - chip * 0.15;
            text.queue(
                &t,
                chip,
                reset_right - tw,
                t_top,
                muted(0.70 * alpha),
                // Half-em slack: an exact measured-width bound can wrap the
                // last glyph onto a clipped second line.
                tw + chip * 0.5,
                surface_w,
                surface_h,
            );
        }
    }

    // Percent, in the window color, rightmost.
    let pct_text = format!("{:.0}%", win.utilization);
    let pw = text.measure_width(&pct_text, stat);
    let p_top = y + (row_h - stat) * 0.5 - stat * 0.15;
    text.queue(
        &pct_text,
        stat,
        pct_right - pw,
        p_top,
        col.with_alpha(alpha),
        pw,
        surface_w,
        surface_h,
    );
}

fn now_epoch() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `5d 21h` for multi-day windows, `2h 41m` under two days, `41m` in
/// the home stretch.
fn fmt_remaining(secs: u64) -> String {
    let mins = secs / 60;
    let hours = mins / 60;
    if hours >= 48 {
        format!("{}d {}h", hours / 24, hours % 24)
    } else if hours >= 1 {
        format!("{}h {}m", hours, mins % 60)
    } else {
        format!("{}m", mins.max(1))
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_bucket_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    x: f32,
    y: f32,
    w: f32,
    scale: f32,
    body: f32,
    stat: f32,
    alpha: f32,
    label: &str,
    cost: f64,
    tokens: u64,
    max_cost: f64,
    cost_col_right: f32,
    tokens_col_right: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let row_h = body * 1.55;
    // Layout: [label .........] [bar .......] [cost-col] [tokens-col]
    // Cost and tokens right-edges are SHARED with the time-window
    // rows above so all three sections line up vertically.
    let label_w = w * 0.30;
    let bar_left = x + label_w;
    // Stop the bar a comfortable gap before the cost column starts so
    // it never collides with the digits.
    let cost_text_max_w = stat * 5.5;
    let bar_right = cost_col_right - cost_text_max_w - 18.0 * scale;
    let bar_full_w = (bar_right - bar_left).max(20.0 * scale);
    let bar_h = (body * 0.30).clamp(5.0 * scale, 9.0 * scale);
    let bar_y = y + (row_h - bar_h) * 0.5;

    let mut shown = label.to_string();
    if shown.chars().count() > 30 {
        shown = shown.chars().take(29).collect::<String>() + "…";
    }
    let l_top = y + (row_h - body) * 0.5 - body * 0.15;
    text.queue(
        &shown,
        body,
        x,
        l_top,
        white(0.92 * alpha),
        label_w - 8.0 * scale,
        surface_w,
        surface_h,
    );

    painter.rect_filled(
        Rect::new(bar_left, bar_y, bar_full_w, bar_h),
        bar_h / 2.0,
        white(0.08 * alpha),
    );
    let frac = (cost / max_cost.max(1e-9)).clamp(0.0, 1.0) as f32;
    if frac > 0.001 {
        painter.rect_filled(
            Rect::new(bar_left, bar_y, bar_full_w * frac, bar_h),
            bar_h / 2.0,
            accent(alpha),
        );
    }

    let s_top = y + (row_h - stat) * 0.5 - stat * 0.15;

    let cost_text = fmt_dollars(cost);
    let cw = text.measure_width(&cost_text, stat);
    text.queue(
        &cost_text,
        stat,
        cost_col_right - cw,
        s_top,
        muted(alpha),
        cw,
        surface_w,
        surface_h,
    );

    let tok_text = fmt_count(tokens);
    let tw = text.measure_width(&tok_text, stat);
    text.queue(
        &tok_text,
        stat,
        tokens_col_right - tw,
        s_top,
        muted(0.85 * alpha),
        tw,
        surface_w,
        surface_h,
    );
}

fn draw_sparkline(painter: &mut Painter, r: Rect, scale: f32, alpha: f32, days: &[DayBucket]) {
    let recent: Vec<&DayBucket> = days.iter().rev().take(30).collect();
    if recent.is_empty() {
        return;
    }
    let max_cost = recent
        .iter()
        .map(|d| d.cost_usd)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    let n = recent.len() as f32;
    let gap = 3.0 * scale;
    let total_gap = gap * (n - 1.0).max(0.0);
    let bar_w = ((r.w - total_gap) / n).max(2.0 * scale);
    for (i, d) in recent.iter().rev().enumerate() {
        let frac = (d.cost_usd / max_cost).clamp(0.0, 1.0) as f32;
        let h = (r.h * frac).max(if d.cost_usd > 0.0 { 2.0 * scale } else { 0.0 });
        let x = r.x + i as f32 * (bar_w + gap);
        let y_top = r.y + r.h - h;
        painter.rect_filled(Rect::new(x, y_top, bar_w, h), bar_w * 0.25, accent(alpha));
    }
}
