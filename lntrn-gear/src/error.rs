//! One error type across the whole crate — transport up through caps.

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// No response within the timeout window.
    Timeout,
    /// Device-reported HID++ error, carrying its error code.
    Protocol(u8),
    /// The device doesn't support the requested capability.
    Unsupported,
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Timeout => write!(f, "timed out waiting for the device"),
            Error::Protocol(code) => write!(f, "device error 0x{code:02x}"),
            Error::Unsupported => write!(f, "capability not supported by this device"),
        }
    }
}
