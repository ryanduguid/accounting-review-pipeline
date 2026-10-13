//! Explicitly check Python's buffered-write and context-manager close failures.
//! Never remove an exclusively-created partial disclosure record on failure.
use std::io::{self, BufWriter, Write};

trait ClosingWrite: Write {
    fn close(self) -> io::Result<()>;
}

fn record_bytes(mut writer: impl ClosingWrite, record: &[u8]) -> io::Result<()> {
    let written = writer.write(record).and_then(|size| {
        if size == record.len() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "incomplete disclosure record write",
            ))
        }
    });
    // A context manager closes even after write fails; its close error wins.
    writer.close().and(written)
}

fn close_file(file: std::fs::File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::IntoRawFd;
        let descriptor = file.into_raw_fd();
        // SAFETY: we own this descriptor, and relinquished File's destructor.
        if unsafe { libc::close(descriptor) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        let handle = file.into_raw_handle();
        // SAFETY: we own this handle, and relinquished File's destructor.
        if unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) } != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

impl ClosingWrite for BufWriter<std::fs::File> {
    fn close(mut self) -> io::Result<()> {
        let flushed = self.flush();
        // Do not silently retry a failed flush in BufWriter's destructor.
        let (file, _) = self.into_parts();
        close_file(file).and(flushed)
    }
}

pub fn record(file: std::fs::File, bytes: &[u8]) -> io::Result<()> {
    record_bytes(BufWriter::new(file), bytes)
}

pub fn text(file: std::fs::File, bytes: &[u8]) -> io::Result<()> {
    let mut writer = BufWriter::new(file);
    let written = writer.write_all(bytes);
    writer.close().and(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct BrokenWriter {
        failure: &'static str,
        closed: Rc<Cell<bool>>,
    }
    impl Write for BrokenWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self.failure {
                "write" => Err(io::Error::other("fabricated write failure")),
                "short" => Ok(bytes.len() - 1),
                _ => Ok(bytes.len()),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl ClosingWrite for BrokenWriter {
        fn close(self) -> io::Result<()> {
            self.closed.set(true);
            if self.failure == "close" {
                Err(io::Error::other("fabricated close failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn failed_short_and_close_writes_never_succeed_and_always_close() {
        for failure in ["write", "short", "close"] {
            let closed = Rc::new(Cell::new(false));
            let writer = BrokenWriter {
                failure,
                closed: closed.clone(),
            };
            assert!(record_bytes(writer, b"fabricated record").is_err());
            assert!(closed.get());
        }
    }
}
