use super::charwidth::char_width;
use super::grid::{Cell, Color8, TerminalState, Wide, ANSI_COLORS};

// ── VTE Performer ───────────────────────────────────────────────────────────
// Bridges the `vte` parser events into our TerminalState grid.

pub struct Performer<'a> {
    pub state: &'a mut TerminalState,
}

impl vte::Perform for Performer<'_> {
    // ── Printable character ──────────────────────────────────────────────

    fn print(&mut self, c: char) {
        let s = &mut *self.state;
        let width = char_width(c);

        // Zero-width characters (combining marks, ZWJ, etc.) — skip
        if width == 0 {
            return;
        }

        // If wrap_next is pending, perform the deferred wrap now
        if s.wrap_next {
            s.wrap_next = false;
            s.cursor_col = 0;
            s.cursor_row += 1;
            if s.cursor_row > s.scroll_bottom {
                s.cursor_row = s.scroll_bottom;
                s.scroll_up();
            }
        }

        // Wide character needs 2 cols — if only 1 left, pad and wrap
        if width == 2 && s.cursor_col + 1 >= s.cols {
            if s.cursor_col < s.cols {
                s.grid[s.cursor_row][s.cursor_col] = s.default_cell();
            }
            s.cursor_col = 0;
            s.cursor_row += 1;
            if s.cursor_row > s.scroll_bottom {
                s.cursor_row = s.scroll_bottom;
                s.scroll_up();
            }
        }

        if s.cursor_row < s.rows && s.cursor_col < s.cols {
            let (fg, bg) = if s.attr_reverse {
                (s.attr_bg, s.attr_fg)
            } else {
                (s.attr_fg, s.attr_bg)
            };
            let bg = if s.attr_reverse && bg.a == 0 {
                s.default_fg
            } else {
                bg
            };

            let wide_flag = if width == 2 { Wide::Head } else { Wide::No };
            s.grid[s.cursor_row][s.cursor_col] = Cell {
                c,
                fg,
                bg,
                bold: s.attr_bold,
                italic: s.attr_italic,
                underline: s.attr_underline,
                wide: wide_flag,
            };
            s.cursor_col += 1;

            // Place continuation (tail) cell for wide characters
            if width == 2 && s.cursor_col < s.cols {
                s.grid[s.cursor_row][s.cursor_col] = Cell {
                    c: ' ',
                    fg,
                    bg,
                    bold: false,
                    italic: false,
                    underline: false,
                    wide: Wide::Tail,
                };
                s.cursor_col += 1;
            }

            // If we just filled the last column, defer the wrap
            if s.cursor_col >= s.cols {
                s.cursor_col = s.cols - 1;
                s.wrap_next = true;
            }
        }
    }

    // ── Control characters ───────────────────────────────────────────────

    fn execute(&mut self, byte: u8) {
        let s = &mut *self.state;
        match byte {
            0x08 => {
                s.wrap_next = false;
                if s.cursor_col > 0 {
                    s.cursor_col -= 1;
                }
            }
            0x09 => {
                s.wrap_next = false;
                let next_tab = (s.cursor_col / 8 + 1) * 8;
                s.cursor_col = next_tab.min(s.cols - 1);
            }
            0x0A | 0x0B | 0x0C => {
                // LF does NOT reset wrap_next — matches xterm behavior.
                // A CR+LF sequence: CR clears it, LF just moves down.
                s.cursor_row += 1;
                if s.cursor_row > s.scroll_bottom {
                    s.cursor_row = s.scroll_bottom;
                    s.scroll_up();
                }
            }
            0x0D => {
                s.wrap_next = false;
                s.cursor_col = 0;
            }
            0x07 => { s.bell = true; }
            _ => {}
        }
    }

    // ── CSI sequences ───────────────────────────────────────────────────

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let s = &mut *self.state;
        let params_vec: Vec<u16> = params.iter().flat_map(|sub| sub.iter().copied()).collect();

        let p = |idx: usize, default: u16| -> u16 {
            params_vec
                .get(idx)
                .copied()
                .filter(|&v| v != 0)
                .unwrap_or(default)
        };

        match action {
            'A' => {
                s.wrap_next = false;
                let n = p(0, 1) as usize;
                s.cursor_row = s.cursor_row.saturating_sub(n);
            }
            'B' => {
                s.wrap_next = false;
                let n = p(0, 1) as usize;
                s.cursor_row = (s.cursor_row + n).min(s.rows - 1);
            }
            'C' => {
                s.wrap_next = false;
                let n = p(0, 1) as usize;
                s.cursor_col = (s.cursor_col + n).min(s.cols - 1);
            }
            'D' => {
                s.wrap_next = false;
                let n = p(0, 1) as usize;
                s.cursor_col = s.cursor_col.saturating_sub(n);
            }
            'E' => {
                s.wrap_next = false;
                let n = p(0, 1) as usize;
                s.cursor_row = (s.cursor_row + n).min(s.rows - 1);
                s.cursor_col = 0;
            }
            'F' => {
                s.wrap_next = false;
                let n = p(0, 1) as usize;
                s.cursor_row = s.cursor_row.saturating_sub(n);
                s.cursor_col = 0;
            }
            'G' => {
                s.wrap_next = false;
                let col = p(0, 1) as usize;
                s.cursor_col = (col - 1).min(s.cols - 1);
            }
            'H' | 'f' => {
                s.wrap_next = false;
                let row = p(0, 1) as usize;
                let col = p(1, 1) as usize;
                s.cursor_row = (row - 1).min(s.rows - 1);
                s.cursor_col = (col - 1).min(s.cols - 1);
            }
            'J' => {
                let mode = p(0, 0);
                match mode {
                    0 => {
                        for c in s.cursor_col..s.cols {
                            s.grid[s.cursor_row][c] = s.default_cell();
                        }
                        for r in (s.cursor_row + 1)..s.rows {
                            for c in 0..s.cols {
                                s.grid[r][c] = s.default_cell();
                            }
                        }
                    }
                    1 => {
                        for r in 0..s.cursor_row {
                            for c in 0..s.cols {
                                s.grid[r][c] = s.default_cell();
                            }
                        }
                        for c in 0..=s.cursor_col.min(s.cols - 1) {
                            s.grid[s.cursor_row][c] = s.default_cell();
                        }
                    }
                    2 | 3 => {
                        for r in 0..s.rows {
                            for c in 0..s.cols {
                                s.grid[r][c] = s.default_cell();
                            }
                        }
                    }
                    _ => {}
                }
            }
            'K' => {
                let mode = p(0, 0);
                match mode {
                    0 => {
                        for c in s.cursor_col..s.cols {
                            s.grid[s.cursor_row][c] = s.default_cell();
                        }
                    }
                    1 => {
                        for c in 0..=s.cursor_col.min(s.cols - 1) {
                            s.grid[s.cursor_row][c] = s.default_cell();
                        }
                    }
                    2 => {
                        for c in 0..s.cols {
                            s.grid[s.cursor_row][c] = s.default_cell();
                        }
                    }
                    _ => {}
                }
            }
            'L' => {
                let n = p(0, 1) as usize;
                for _ in 0..n {
                    if s.cursor_row <= s.scroll_bottom {
                        if s.scroll_bottom < s.rows {
                            s.grid.remove(s.scroll_bottom);
                        }
                        s.grid.insert(s.cursor_row, vec![s.default_cell(); s.cols]);
                    }
                }
            }
            'M' => {
                let n = p(0, 1) as usize;
                for _ in 0..n {
                    if s.cursor_row <= s.scroll_bottom {
                        s.grid.remove(s.cursor_row);
                        s.grid
                            .insert(s.scroll_bottom, vec![s.default_cell(); s.cols]);
                    }
                }
            }
            'P' => {
                let n = p(0, 1) as usize;
                let def = s.default_cell();
                let row = &mut s.grid[s.cursor_row];
                for _ in 0..n {
                    if s.cursor_col < row.len() {
                        row.remove(s.cursor_col);
                        row.push(def.clone());
                    }
                }
            }
            'S' => {
                let n = p(0, 1) as usize;
                for _ in 0..n {
                    s.scroll_up();
                }
            }
            'T' => {
                let n = p(0, 1) as usize;
                for _ in 0..n {
                    s.scroll_down();
                }
            }
            'X' => {
                let n = p(0, 1) as usize;
                for i in 0..n {
                    let col = s.cursor_col + i;
                    if col < s.cols {
                        s.grid[s.cursor_row][col] = s.default_cell();
                    }
                }
            }
            '@' => {
                let n = p(0, 1) as usize;
                let def = s.default_cell();
                let row = &mut s.grid[s.cursor_row];
                for _ in 0..n {
                    if s.cursor_col < row.len() {
                        row.insert(s.cursor_col, def.clone());
                        row.truncate(s.cols);
                    }
                }
            }
            'Z' => {
                let n = p(0, 1) as usize;
                for _ in 0..n {
                    if s.cursor_col > 0 {
                        s.cursor_col = ((s.cursor_col - 1) / 8) * 8;
                    }
                }
            }
            'm' => {
                apply_sgr(s, &params_vec);
            }
            'r' => {
                s.wrap_next = false;
                let top = p(0, 1) as usize;
                let bottom = p(1, s.rows as u16) as usize;
                s.scroll_top = (top - 1).min(s.rows - 1);
                s.scroll_bottom = (bottom - 1).min(s.rows - 1);
                s.cursor_row = 0;
                s.cursor_col = 0;
            }
            'c' => {
                if intermediates == [b'>'] {
                    // Secondary Device Attributes (DA2) — identify as Lantern 0.1.0
                    // Format: CSI > Pp ; Pv ; Pc c
                    // Pp=1 (VT100 family), Pv=100 (version 0.1.0 as int), Pc=0
                    s.pending_responses.push(b"\x1b[>1;100;0c".to_vec());
                } else if intermediates.is_empty() || intermediates == [b'?'] {
                    // Primary Device Attributes (DA1) — report as VT220
                    // (VT220 is more appropriate for a modern terminal with 256-color)
                    s.pending_responses.push(b"\x1b[?62;22c".to_vec());
                }
            }
            's' => {
                if intermediates.is_empty() {
                    s.saved_cursor = Some((s.cursor_row, s.cursor_col));
                }
            }
            'u' => {
                s.wrap_next = false;
                if let Some((row, col)) = s.saved_cursor {
                    s.cursor_row = row.min(s.rows.saturating_sub(1));
                    s.cursor_col = col.min(s.cols.saturating_sub(1));
                }
            }
            'n' => {
                // Device Status Report (DSR)
                match p(0, 0) {
                    5 => {
                        // Terminal OK
                        s.pending_responses.push(b"\x1b[0n".to_vec());
                    }
                    6 => {
                        // Cursor Position Report
                        let response = format!("\x1b[{};{}R", s.cursor_row + 1, s.cursor_col + 1);
                        s.pending_responses.push(response.into_bytes());
                    }
                    _ => {}
                }
            }
            'h' | 'l' => {
                let set = action == 'h';
                if intermediates == [b'?'] {
                    for &p in &params_vec {
                        match p {
                            1 => {
                                // DECCKM — application cursor keys
                                s.application_cursor = set;
                            }
                            25 => {
                                // DECTCEM — cursor visibility
                                s.cursor_hidden = !set;
                            }
                            1049 => {
                                if set {
                                    s.enter_alt_screen();
                                } else {
                                    s.leave_alt_screen();
                                }
                            }
                            1047 | 47 => {
                                if set {
                                    s.enter_alt_screen();
                                } else {
                                    s.leave_alt_screen();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            'q' => {
                if intermediates == [b'>'] {
                    // XTVERSION — report terminal name and version
                    // Response: DCS > | Lantern 0.1.0 ST
                    s.pending_responses.push(b"\x1bP>|Lantern 0.1.0\x1b\\".to_vec());
                } else if intermediates == [b' '] {
                    // DECSCUSR — set cursor shape
                    s.cursor_shape = p(0, 0) as u8;
                }
            }
            '`' => {
                s.wrap_next = false;
                let col = p(0, 1) as usize;
                s.cursor_col = (col - 1).min(s.cols - 1);
            }
            'd' => {
                s.wrap_next = false;
                let row = p(0, 1) as usize;
                s.cursor_row = (row - 1).min(s.rows - 1);
            }
            _ => {}
        }
    }

    // ── ESC sequences ───────────────────────────────────────────────────

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        let s = &mut *self.state;
        match byte {
            b'c' => {
                *s = TerminalState::new(s.cols, s.rows);
            }
            b'7' => {
                // DECSC — save cursor
                s.saved_cursor = Some((s.cursor_row, s.cursor_col));
            }
            b'8' => {
                // DECRC — restore cursor
                s.wrap_next = false;
                if let Some((row, col)) = s.saved_cursor {
                    s.cursor_row = row.min(s.rows.saturating_sub(1));
                    s.cursor_col = col.min(s.cols.saturating_sub(1));
                }
            }
            b'D' => {
                if s.cursor_row >= s.scroll_bottom {
                    s.scroll_up();
                } else {
                    s.cursor_row += 1;
                }
            }
            b'E' => {
                s.cursor_col = 0;
                if s.cursor_row >= s.scroll_bottom {
                    s.scroll_up();
                } else {
                    s.cursor_row += 1;
                }
            }
            b'M' => {
                if s.cursor_row <= s.scroll_top {
                    s.scroll_down();
                } else {
                    s.cursor_row -= 1;
                }
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        match params[0] {
            b"0" | b"2" => {
                // Set window/icon title
                if params.len() > 1 {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        self.state.title = Some(title.to_string());
                    }
                }
            }
            b"9" => {
                // OSC 9: Desktop notification (iTerm2/common style)
                // Format: ESC ] 9 ; <message> ST
                if params.len() > 1 {
                    if let Ok(msg) = std::str::from_utf8(params[1]) {
                        self.state.pending_notifications.push(("Terminal".to_string(), msg.to_string()));
                    }
                }
            }
            b"7" => {
                // OSC 7: report working directory — file://<host>/<path>
                if params.len() > 1 {
                    if let Ok(uri) = std::str::from_utf8(params[1]) {
                        let path = if let Some(rest) = uri.strip_prefix("file://") {
                            // Skip hostname (everything up to the next /)
                            rest.find('/').map(|i| &rest[i..]).unwrap_or(rest)
                        } else {
                            uri
                        };
                        self.state.osc7_cwd = Some(path.to_string());
                    }
                }
            }
            b"99" => {
                // OSC 99: Kitty desktop notification protocol
                // Format: ESC ] 99 ; <metadata> ; <payload> ST
                // metadata is colon-separated key=value pairs:
                //   i=<id>  d=0|1 (0=more data coming, 1=done)  p=title|body  a=focus
                if params.len() < 2 {
                    return;
                }
                let meta = std::str::from_utf8(params[1]).unwrap_or("");
                let payload = if params.len() > 2 {
                    std::str::from_utf8(params[2]).unwrap_or("")
                } else {
                    ""
                };

                let mut done = false;
                let mut part = ""; // "title" or "body"
                for kv in meta.split(':') {
                    if let Some(v) = kv.strip_prefix("d=") {
                        done = v == "1";
                    } else if let Some(v) = kv.strip_prefix("p=") {
                        part = v;
                    }
                }

                match part {
                    "title" => self.state.osc99_title = payload.to_string(),
                    "body" => self.state.osc99_body = payload.to_string(),
                    _ => {}
                }

                if done {
                    let title = std::mem::take(&mut self.state.osc99_title);
                    let body = std::mem::take(&mut self.state.osc99_body);
                    if !title.is_empty() || !body.is_empty() {
                        self.state.pending_notifications.push((title, body));
                    }
                }
            }
            b"777" => {
                // OSC 777: rxvt-unicode notification
                // Format: ESC ] 777 ; notify ; <title> ; <body> ST
                if params.len() >= 3 {
                    if let Ok(cmd) = std::str::from_utf8(params[1]) {
                        if cmd == "notify" {
                            let title = std::str::from_utf8(params[2]).unwrap_or("Terminal").to_string();
                            let body = if params.len() > 3 {
                                std::str::from_utf8(params[3]).unwrap_or("").to_string()
                            } else {
                                String::new()
                            };
                            self.state.pending_notifications.push((title, body));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
    }

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}
}

// ── SGR (Select Graphic Rendition) handler ──────────────────────────────────

fn apply_sgr(s: &mut TerminalState, params: &[u16]) {
    if params.is_empty() {
        reset_attrs(s);
        return;
    }

    let mut i = 0;
    while i < params.len() {
        match params[i] {
            0 => reset_attrs(s),
            1 => s.attr_bold = true,
            3 => s.attr_italic = true,
            4 => s.attr_underline = true,
            7 => s.attr_reverse = true,
            22 => s.attr_bold = false,
            23 => s.attr_italic = false,
            24 => s.attr_underline = false,
            27 => s.attr_reverse = false,
            30..=37 => {
                s.attr_fg = ANSI_COLORS[(params[i] - 30) as usize];
            }
            39 => {
                s.attr_fg = s.default_fg;
            }
            40..=47 => {
                s.attr_bg = ANSI_COLORS[(params[i] - 40) as usize];
            }
            49 => {
                s.attr_bg = s.default_bg;
            }
            90..=97 => {
                s.attr_fg = ANSI_COLORS[(params[i] - 90 + 8) as usize];
            }
            100..=107 => {
                s.attr_bg = ANSI_COLORS[(params[i] - 100 + 8) as usize];
            }
            38 => {
                if i + 1 < params.len() {
                    match params[i + 1] {
                        5 => {
                            if i + 2 < params.len() {
                                s.attr_fg = color_from_256(params[i + 2]);
                                i += 2;
                            }
                        }
                        2 => {
                            if i + 4 < params.len() {
                                s.attr_fg = Color8::from_rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                );
                                i += 4;
                            }
                        }
                        _ => {}
                    }
                }
            }
            48 => {
                if i + 1 < params.len() {
                    match params[i + 1] {
                        5 => {
                            if i + 2 < params.len() {
                                s.attr_bg = color_from_256(params[i + 2]);
                                i += 2;
                            }
                        }
                        2 => {
                            if i + 4 < params.len() {
                                s.attr_bg = Color8::from_rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                );
                                i += 4;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn reset_attrs(s: &mut TerminalState) {
    s.attr_fg = s.default_fg;
    s.attr_bg = s.default_bg;
    s.attr_bold = false;
    s.attr_italic = false;
    s.attr_underline = false;
    s.attr_reverse = false;
}

fn color_from_256(idx: u16) -> Color8 {
    match idx {
        0..=15 => ANSI_COLORS[idx as usize],
        16..=231 => {
            let n = idx - 16;
            let b = (n % 6) as u8;
            let g = ((n / 6) % 6) as u8;
            let r = (n / 36) as u8;
            let to_val = |c: u8| if c == 0 { 0u8 } else { 55 + 40 * c };
            Color8::from_rgb(to_val(r), to_val(g), to_val(b))
        }
        232..=255 => {
            let v = (8 + 10 * (idx - 232)) as u8;
            Color8::from_rgb(v, v, v)
        }
        _ => Color8::from_rgb(236, 236, 236),
    }
}
