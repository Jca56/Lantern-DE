//! How a child process ended.

use std::fmt;

use nix::sys::signal::Signal;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exit {
    Code(i32),
    /// The signal that killed it, and whether it dumped core.
    Signal(Signal, bool),
}

impl Exit {
    /// It finished by itself and reported no error.
    pub fn is_clean(self) -> bool {
        self == Exit::Code(0)
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Exit::Code(code) => write!(f, "exit code {code}"),
            Exit::Signal(signal, false) => write!(f, "killed by {signal}"),
            Exit::Signal(signal, true) => write!(f, "killed by {signal}, core dumped"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEGV: Exit = Exit::Signal(Signal::SIGSEGV, true);

    #[test]
    fn exits_read_plainly() {
        assert_eq!(Exit::Code(1).to_string(), "exit code 1");
        assert_eq!(Exit::Signal(Signal::SIGTERM, false).to_string(), "killed by SIGTERM");
        assert_eq!(SEGV.to_string(), "killed by SIGSEGV, core dumped");
        assert!(Exit::Code(0).is_clean());
        assert!(!Exit::Code(1).is_clean());
        assert!(!SEGV.is_clean());
    }
}
