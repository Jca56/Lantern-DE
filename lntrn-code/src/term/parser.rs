//! The escape-sequence parser: bytes in, actions out. A small state
//! machine after the VT500 model: ground, escape, CSI with parameters
//! and intermediates, OSC strings, and the strings (DCS, APC, PM, SOS)
//! that are skipped whole. UTF-8 is decoded on the way.

/// What a stretch of bytes meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Print(char),
    /// A C0 control (BEL, BS, HT, LF, CR, ...).
    Execute(u8),
    Csi { params: Vec<u16>, private: Option<u8>, intermediate: Option<u8>, final_byte: u8 },
    Esc { intermediate: Option<u8>, final_byte: u8 },
    Osc(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    Osc,
    /// DCS, SOS, PM, APC: ignored until the string terminator.
    IgnoreString,
}

const MAX_PARAMS: usize = 32;

pub struct Parser {
    state: State,
    params: Vec<u16>,
    param: u32,
    has_param: bool,
    private: Option<u8>,
    intermediate: Option<u8>,
    osc: Vec<u8>,
    /// An ESC arrived inside a string: the next `\` ends it.
    esc_in_string: bool,
    utf8: Vec<u8>,
    utf8_need: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self { state: State::Ground, params: Vec::new(), param: 0, has_param: false, private: None, intermediate: None, osc: Vec::new(), esc_in_string: false, utf8: Vec::new(), utf8_need: 0 }
    }

    pub fn feed(&mut self, bytes: &[u8], mut emit: impl FnMut(Action)) {
        for &b in bytes {
            self.byte(b, &mut emit);
        }
    }

    fn clear(&mut self) {
        self.params.clear();
        self.param = 0;
        self.has_param = false;
        self.private = None;
        self.intermediate = None;
    }

    fn push_param(&mut self) {
        if self.params.len() < MAX_PARAMS {
            self.params.push(self.param.min(u16::MAX as u32) as u16);
        }
        self.param = 0;
        self.has_param = false;
    }

    fn byte(&mut self, b: u8, emit: &mut impl FnMut(Action)) {
        // A UTF-8 sequence in progress takes its continuation bytes first.
        if self.utf8_need > 0 {
            if (0x80..0xC0).contains(&b) {
                self.utf8.push(b);
                self.utf8_need -= 1;
                if self.utf8_need == 0 {
                    let c = std::str::from_utf8(&self.utf8).ok().and_then(|s| s.chars().next()).unwrap_or('\u{FFFD}');
                    emit(Action::Print(c));
                    self.utf8.clear();
                }
                return;
            }
            // Broken sequence: drop it and treat this byte afresh.
            self.utf8.clear();
            self.utf8_need = 0;
            emit(Action::Print('\u{FFFD}'));
        }
        // ESC anywhere (outside strings) starts over; CAN/SUB abort.
        if b == 0x1B && !matches!(self.state, State::Osc | State::IgnoreString) {
            self.state = State::Escape;
            self.clear();
            return;
        }
        match self.state {
            State::Ground => match b {
                0x00..=0x1F => emit(Action::Execute(b)),
                0x7F => {}
                0x20..=0x7E => emit(Action::Print(b as char)),
                0xC0..=0xDF => self.start_utf8(b, 1),
                0xE0..=0xEF => self.start_utf8(b, 2),
                0xF0..=0xF7 => self.start_utf8(b, 3),
                _ => emit(Action::Print('\u{FFFD}')),
            },
            State::Escape => match b {
                b'[' => {
                    self.state = State::CsiEntry;
                    self.clear();
                }
                b']' => {
                    self.state = State::Osc;
                    self.osc.clear();
                    self.esc_in_string = false;
                }
                b'P' | b'X' | b'^' | b'_' => {
                    self.state = State::IgnoreString;
                    self.esc_in_string = false;
                }
                0x20..=0x2F => {
                    self.intermediate = Some(b);
                    self.state = State::EscapeIntermediate;
                }
                0x30..=0x7E => {
                    emit(Action::Esc { intermediate: None, final_byte: b });
                    self.state = State::Ground;
                }
                0x18 | 0x1A => self.state = State::Ground,
                0x00..=0x1F => emit(Action::Execute(b)),
                _ => self.state = State::Ground,
            },
            State::EscapeIntermediate => match b {
                0x20..=0x2F => self.intermediate = Some(b),
                0x30..=0x7E => {
                    emit(Action::Esc { intermediate: self.intermediate, final_byte: b });
                    self.state = State::Ground;
                }
                0x18 | 0x1A => self.state = State::Ground,
                0x00..=0x1F => emit(Action::Execute(b)),
                _ => self.state = State::Ground,
            },
            State::CsiEntry | State::CsiParam | State::CsiIntermediate => match b {
                b'0'..=b'9' if self.state != State::CsiIntermediate => {
                    self.param = self.param.saturating_mul(10).saturating_add((b - b'0') as u32);
                    self.has_param = true;
                    self.state = State::CsiParam;
                }
                b';' | b':' if self.state != State::CsiIntermediate => {
                    self.push_param();
                    self.state = State::CsiParam;
                }
                b'<'..=b'?' if self.state == State::CsiEntry => {
                    self.private = Some(b);
                    self.state = State::CsiParam;
                }
                0x20..=0x2F => {
                    self.intermediate = Some(b);
                    self.state = State::CsiIntermediate;
                }
                0x40..=0x7E => {
                    if self.has_param || !self.params.is_empty() {
                        self.push_param();
                    }
                    let params = std::mem::take(&mut self.params);
                    emit(Action::Csi { params, private: self.private, intermediate: self.intermediate, final_byte: b });
                    self.state = State::Ground;
                    self.clear();
                }
                0x18 | 0x1A => self.state = State::Ground,
                0x00..=0x1F => emit(Action::Execute(b)),
                0x7F => {}
                _ => self.state = State::CsiIgnore,
            },
            State::CsiIgnore => match b {
                0x40..=0x7E | 0x18 | 0x1A => self.state = State::Ground,
                0x00..=0x1F => emit(Action::Execute(b)),
                _ => {}
            },
            State::Osc => {
                if self.esc_in_string {
                    self.esc_in_string = false;
                    if b == b'\\' {
                        emit(Action::Osc(std::mem::take(&mut self.osc)));
                        self.state = State::Ground;
                        return;
                    }
                    // A lone ESC in an OSC restarts parsing.
                    self.state = State::Escape;
                    self.clear();
                    return self.byte(b, emit);
                }
                match b {
                    0x07 => {
                        emit(Action::Osc(std::mem::take(&mut self.osc)));
                        self.state = State::Ground;
                    }
                    0x1B => self.esc_in_string = true,
                    0x18 | 0x1A => self.state = State::Ground,
                    _ => {
                        if self.osc.len() < 4096 {
                            self.osc.push(b);
                        }
                    }
                }
            }
            State::IgnoreString => {
                if self.esc_in_string {
                    self.esc_in_string = false;
                    if b == b'\\' {
                        self.state = State::Ground;
                    } else {
                        self.state = State::Escape;
                        self.clear();
                        self.byte(b, emit);
                    }
                } else if b == 0x1B {
                    self.esc_in_string = true;
                } else if b == 0x07 || b == 0x18 || b == 0x1A {
                    self.state = State::Ground;
                }
            }
        }
    }

    fn start_utf8(&mut self, b: u8, need: usize) {
        self.utf8.clear();
        self.utf8.push(b);
        self.utf8_need = need;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Vec<Action> {
        let mut out = Vec::new();
        Parser::new().feed(bytes, |a| out.push(a));
        out
    }

    #[test]
    fn text_controls_and_utf8() {
        let a = parse(b"a\r\n\xC3\xA9\xF0\x9F\xA6\x8A");
        assert_eq!(a, vec![Action::Print('a'), Action::Execute(b'\r'), Action::Execute(b'\n'), Action::Print('é'), Action::Print('🦊')]);
        assert_eq!(parse(b"\xC3x"), vec![Action::Print('\u{FFFD}'), Action::Print('x')], "a broken sequence is replaced");
    }

    #[test]
    fn csi_forms() {
        assert_eq!(parse(b"\x1b[2;5H"), vec![Action::Csi { params: vec![2, 5], private: None, intermediate: None, final_byte: b'H' }]);
        assert_eq!(parse(b"\x1b[?1049h"), vec![Action::Csi { params: vec![1049], private: Some(b'?'), intermediate: None, final_byte: b'h' }]);
        assert_eq!(parse(b"\x1b[m"), vec![Action::Csi { params: vec![], private: None, intermediate: None, final_byte: b'm' }]);
        assert_eq!(parse(b"\x1b[38:2:1:2:3m"), vec![Action::Csi { params: vec![38, 2, 1, 2, 3], private: None, intermediate: None, final_byte: b'm' }]);
        assert_eq!(parse(b"\x1b[;5m"), vec![Action::Csi { params: vec![0, 5], private: None, intermediate: None, final_byte: b'm' }]);
        assert_eq!(parse(b"\x1b[ q"), vec![Action::Csi { params: vec![], private: None, intermediate: Some(b' '), final_byte: b'q' }]);
        assert_eq!(parse(b"\x1b(0"), vec![Action::Esc { intermediate: Some(b'('), final_byte: b'0' }]);
        assert_eq!(parse(b"\x1bM"), vec![Action::Esc { intermediate: None, final_byte: b'M' }]);
    }

    #[test]
    fn strings() {
        assert_eq!(parse(b"\x1b]0;hello\x07x"), vec![Action::Osc(b"0;hello".to_vec()), Action::Print('x')]);
        assert_eq!(parse(b"\x1b]2;t\x1b\\y"), vec![Action::Osc(b"2;t".to_vec()), Action::Print('y')]);
        assert_eq!(parse(b"\x1bPjunk\x1b\\z"), vec![Action::Print('z')], "DCS is skipped");
        // An escape mid-CSI starts over.
        assert_eq!(parse(b"\x1b[12\x1b[3A"), vec![Action::Csi { params: vec![3], private: None, intermediate: None, final_byte: b'A' }]);
    }
}
