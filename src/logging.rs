//! Tracing and panic output that must not take the process down.
//!
//! Rust's `eprint!` panics on `ErrorKind::BrokenPipe`. tracing-subscriber
//! then calls `eprintln!` whenever its writer returns `Err`. Release builds
//! use `panic = "abort"`, so a log line after the launching terminal closes
//! its pipe SIGABRTs the daemon.

use std::io::{self, Write};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Copy, Debug, Default)]
pub struct StderrWriter;

impl Write for StderrWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_best_effort(&mut io::stderr(), buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for StderrWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

fn is_unusable(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
    )
}

/// Always returns `Ok`. A writer `Err` makes tracing-subscriber `eprintln!`,
/// which panics on a closed stderr.
pub(crate) fn write_best_effort<W: Write + ?Sized>(out: &mut W, buf: &[u8]) -> io::Result<usize> {
    let mut rest = buf;
    while !rest.is_empty() {
        match out.write(rest) {
            Ok(0) => return Ok(buf.len()),
            Ok(n) => rest = &rest[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if is_unusable(&e) => return Ok(buf.len()),
            Err(_) => return Ok(buf.len()),
        }
    }
    Ok(buf.len())
}

pub fn init() {
    std::panic::set_hook(Box::new(|info| {
        let mut w = StderrWriter;
        let _ = writeln!(w, "{info}");
    }));

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(StderrWriter)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fail(io::ErrorKind);

    impl Write for Fail {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let _ = buf;
            Err(io::Error::new(self.0, "nope"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct PartialThenPipe {
        wrote: bool,
    }

    impl Write for PartialThenPipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if !self.wrote {
                self.wrote = true;
                Ok(1.min(buf.len()))
            } else {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_counts_as_written() {
        let n = write_best_effort(&mut Fail(io::ErrorKind::BrokenPipe), b"hello").unwrap();
        assert_eq!(n, 5);
    }

    #[test]
    fn other_io_errors_do_not_fail() {
        let n = write_best_effort(&mut Fail(io::ErrorKind::PermissionDenied), b"x").unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn successful_write_passes_through() {
        let mut buf = Vec::new();
        assert_eq!(write_best_effort(&mut buf, b"abc").unwrap(), 3);
        assert_eq!(buf, b"abc");
    }

    #[test]
    fn partial_write_then_broken_pipe_is_ok() {
        let mut w = PartialThenPipe { wrote: false };
        assert_eq!(write_best_effort(&mut w, b"hello").unwrap(), 5);
    }
}
