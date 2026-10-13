//! Copied from elizabeth-anne-alexander/src/main.rs: Python OS argv and text streams.
use evatt::{cli, pypath, stdio};

/// A Python 3.14 text stream: `\r\n` for each newline on
/// Windows, and lone surrogates written as stdout's surrogateescape (the
/// original byte) or stderr's backslashreplace (`\udcXX`) give them.
struct PythonText<W: std::io::Write> {
    inner: W,
    stderr: bool,
    failed_encoding: bool,
    failed_write: bool,
    encoding: stdio::Encoding,
    wrote_bom: bool,
    line_buffering: bool,
}

impl<W: std::io::Write> std::io::Write for PythonText<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Every write comes from a formatted &str, so it is whole UTF-8.
        let text = String::from_utf8_lossy(buf);
        let mut out = stdio::encode(&text, self.stderr, &self.encoding).map_err(|_| {
            self.failed_encoding = true;
            std::io::Error::new(std::io::ErrorKind::InvalidData, "stdout cannot encode supplied text")
        })?;
        if matches!(self.encoding, stdio::Encoding::Utf8Sig) && !self.wrote_bom {
            out.splice(0..0, b"\xef\xbb\xbf".iter().copied());
            self.wrote_bom = true;
        }
        self.inner.write_all(&out).inspect_err(|_| self.failed_write = true)?;
        if self.line_buffering && text.contains(['\n', '\r']) {
            self.flush()?;
        }
        Ok(buf.len())
    }

    fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        self.write_all(args.to_string().as_bytes())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush().inspect_err(|_| self.failed_write = true)
    }
}

fn capacity() -> usize {
    // CPython reads PYTHONUNBUFFERED with its integer flag rule ("00" keeps buffering).
    if stdio::env_flag("PYTHONUNBUFFERED") { 0 } else { 8192 }
}

fn finish<W: std::io::Write, E: std::io::Write>(
    stdout: &mut PythonText<W>,
    stderr: &mut PythonText<E>,
    outcome: cli::Outcome,
) -> i32 {
    use std::io::Write;
    let failed_flush = stdout.flush().is_err() | stderr.flush().is_err();
    if failed_flush {
        let _ = writeln!(stderr, "error: cannot flush CLI diagnostic");
        return 120;
    }
    if stdout.failed_encoding {
        let _ = writeln!(stderr, "error: stdout cannot encode supplied text");
        return 1;
    }
    if (stdout.failed_write || stderr.failed_write) && !outcome.argparse {
        let _ = writeln!(stderr, "error: cannot write CLI diagnostic");
        return 1;
    }
    // argparse._print_message deliberately catches OSError; unbuffered help
    // therefore keeps its requested exit even when the pipe has no reader.
    outcome.code
}

fn main() {
    // An unexpected failure exits 1, as an uncaught exception did in the Python release.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("error: {info}");
        std::process::exit(1);
    }));
    // sys.argv as Python decodes it: undecodable bytes or UTF-16 stay as surrogates.
    // argv is the user's own command line, decoded as Python decodes sys.argv; it grants nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    // Settle the streams before argv is decoded or any command runs. A configuration
    // CPython would encode differently, or refuse to start under, is refused here.
    if let Err(message) = stdio::configure() {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "error: {message}");
        std::process::exit(1);
    }
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| pypath::decode_os(&a)).collect();
    // A deeply nested JSON input recurses; give it the room CPython's own limit allows.
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            use std::io::IsTerminal;
            let mut stdout = PythonText {
                inner: std::io::BufWriter::with_capacity(capacity(), std::io::stdout().lock()),
                stderr: false,
                failed_encoding: false,
                failed_write: false,
                encoding: stdio::encoding(false).unwrap_or(stdio::Encoding::Utf8),
                wrote_bom: false,
                line_buffering: std::io::stdout().is_terminal(),
            };
            let mut stderr = PythonText {
                inner: std::io::BufWriter::with_capacity(capacity(), std::io::stderr().lock()),
                stderr: true,
                failed_encoding: false,
                failed_write: false,
                encoding: stdio::encoding(true).unwrap_or(stdio::Encoding::Utf8),
                wrote_bom: false,
                line_buffering: true,
            };
            let outcome = cli::run(&args, &mut stdout, &mut stderr);
            finish(&mut stdout, &mut stderr, outcome)
        })
        .expect("the worker thread starts")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
