/// Pure calculator engine — no rendering, just math and state.

#[derive(Clone, Copy, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    pub fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "\u{2212}",
            Op::Mul => "\u{00d7}",
            Op::Div => "\u{00f7}",
        }
    }

    fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            Op::Div => {
                if b == 0.0 {
                    f64::NAN
                } else {
                    a / b
                }
            }
        }
    }
}

pub struct Calculator {
    /// The text currently being entered / displayed.
    pub display: String,
    /// The expression string shown above the result (e.g. "12 + 4").
    pub expression: String,
    /// Accumulated value from previous operations.
    accumulator: Option<f64>,
    /// Pending operator waiting for the second operand.
    pending_op: Option<Op>,
    /// True right after `=` or an operator is pressed (next digit replaces display).
    start_new: bool,
    /// True if an error occurred (division by zero, etc.).
    pub error: bool,
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            display: "0".into(),
            expression: String::new(),
            accumulator: None,
            pending_op: None,
            start_new: true,
            error: false,
        }
    }

    pub fn press_digit(&mut self, d: char) {
        if self.error {
            self.clear();
        }
        if self.start_new {
            self.display.clear();
            self.start_new = false;
        }
        // Prevent leading zeros (but allow "0.")
        if self.display == "0" && d != '.' {
            self.display.clear();
        }
        // Only one decimal point
        if d == '.' && self.display.contains('.') {
            return;
        }
        // Start with "0." if pressing dot on empty
        if d == '.' && self.display.is_empty() {
            self.display.push('0');
        }
        self.display.push(d);
    }

    pub fn press_operator(&mut self, op: Op) {
        if self.error {
            return;
        }
        let current = self.display_value();
        // Build up the expression string
        if self.accumulator.is_none() {
            self.expression = format!("{} {} ", format_number(current), op.symbol());
        } else if !self.start_new {
            // Chain: evaluate pending, then set new op
            self.evaluate_pending(current);
            if self.error {
                return;
            }
            let acc = self.accumulator.unwrap_or(current);
            self.expression = format!("{} {} ", format_number(acc), op.symbol());
        } else {
            // Just change the operator
            self.expression = format!(
                "{} {} ",
                format_number(self.accumulator.unwrap_or(current)),
                op.symbol()
            );
        }

        if self.accumulator.is_none() {
            self.accumulator = Some(current);
        }
        self.pending_op = Some(op);
        self.start_new = true;
    }

    pub fn press_equals(&mut self) {
        if self.error {
            return;
        }
        let current = self.display_value();
        if let Some(op) = self.pending_op {
            let acc = self.accumulator.unwrap_or(0.0);
            // Show full expression
            self.expression = format!(
                "{} {} {}",
                format_number(acc),
                op.symbol(),
                format_number(current)
            );
            let result = op.apply(acc, current);
            if result.is_nan() || result.is_infinite() {
                self.display = "Error".into();
                self.error = true;
            } else {
                self.display = format_number(result);
                self.accumulator = Some(result);
            }
        }
        self.pending_op = None;
        self.start_new = true;
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn press_negate(&mut self) {
        if self.error || self.display == "0" {
            return;
        }
        if self.display.starts_with('-') {
            self.display.remove(0);
        } else {
            self.display.insert(0, '-');
        }
    }

    pub fn press_percent(&mut self) {
        if self.error {
            return;
        }
        let val = self.display_value() / 100.0;
        self.display = format_number(val);
        self.start_new = true;
    }

    pub fn press_backspace(&mut self) {
        if self.error || self.start_new {
            return;
        }
        self.display.pop();
        if self.display.is_empty() || self.display == "-" {
            self.display = "0".into();
            self.start_new = true;
        }
    }

    fn display_value(&self) -> f64 {
        self.display.parse::<f64>().unwrap_or(0.0)
    }

    fn evaluate_pending(&mut self, current: f64) {
        if let (Some(acc), Some(op)) = (self.accumulator, self.pending_op) {
            let result = op.apply(acc, current);
            if result.is_nan() || result.is_infinite() {
                self.display = "Error".into();
                self.error = true;
            } else {
                self.display = format_number(result);
                self.accumulator = Some(result);
            }
        }
    }
}

/// Format a number nicely — no trailing zeros, reasonable precision.
fn format_number(n: f64) -> String {
    if n == n.floor() && n.abs() < 1e15 {
        format!("{:.0}", n)
    } else {
        // Up to 10 decimal digits, strip trailing zeros
        let s = format!("{:.10}", n);
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    }
}
