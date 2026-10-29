use grant_acquittal_workpapers::{cli, oserror, pytext};

/// A Python 3.15 text stream in UTF-8 mode: `\r\n` for each newline on
/// Windows, and lone surrogates written as stdout's surrogateescape (the
/// original byte) or stderr's backslashreplace (`\udcXX`) give them.
struct PythonText<W: std::io::Write> {
    inner: W,
    stderr: bool,
    /// A write failed, leaving its bytes pending in a buffered Python stream.
    failed: bool,
}

impl<W: std::io::Write> std::io::Write for PythonText<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Every write comes from a formatted &str, so it is whole UTF-8.
        let text = String::from_utf8_lossy(buf);
        let mut out = Vec::with_capacity(buf.len());
        for cp in pytext::code_points(&text) {
            match char::from_u32(cp) {
                None if (0xDC80..=0xDCFF).contains(&cp) && !self.stderr => out.push((cp - 0xDC00) as u8),
                None => out.extend_from_slice(format!("\\u{cp:04x}").as_bytes()),
                Some('\n') if cfg!(windows) => out.extend_from_slice(b"\r\n"),
                Some(ch) => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
            }
        }
        self.inner.write_all(&out).inspect_err(|_| self.failed = true)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// `PYTHONUNBUFFERED` as CPython reads it: set and not an integer equal to 0.
fn python_unbuffered() -> bool {
    std::env::var("PYTHONUNBUFFERED").is_ok_and(|value| {
        !value.is_empty()
            && value
                .trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
                .parse::<i32>()
                != Ok(0)
    })
}

/// stdout as the script had it: line-buffered on a terminal, written straight
/// to the handle with `PYTHONUNBUFFERED`, and otherwise held in a 128 KiB
/// buffer (CPython 3.15's `io.DEFAULT_BUFFER_SIZE`) until it fills or the
/// process exits.
fn python_stdout(unbuffered: bool) -> Box<dyn std::io::Write + Send> {
    use std::io::IsTerminal;
    let stdout = std::io::stdout();
    if stdout.is_terminal() {
        return Box::new(std::io::LineWriter::new(stdout));
    }
    if unbuffered {
        #[cfg(unix)]
        let handle = std::os::fd::AsFd::as_fd(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from);
        #[cfg(windows)]
        let handle = std::os::windows::io::AsHandle::as_handle(&stdout)
            .try_clone_to_owned()
            .map(std::fs::File::from);
        if let Ok(file) = handle {
            return Box::new(file);
        }
    }
    Box::new(std::io::BufWriter::with_capacity(128 * 1024, stdout))
}

fn main() {
    // An unexpected failure exits 1, as an uncaught exception did in the Python script.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("grant-workpaper: internal error: {info}");
        std::process::exit(1);
    }));
    // argv is the user's own command line, decoded as Python decodes sys.argv; it grants nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    let mut argv = std::env::args_os();
    // argparse names the program after the basename of argv[0].
    let prog = argv
        .next()
        .and_then(|first| std::path::Path::new(&first).file_name().map(oserror::decode_os))
        .unwrap_or_else(|| "grant-workpaper".into());
    let args: Vec<String> = argv.map(|arg| oserror::decode_os(&arg)).collect();
    let cwd = std::env::current_dir().expect("the working directory must exist");
    // Deeply nested JSON recurses; give it the room CPython's own limit allows.
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            use std::io::Write;
            let unbuffered = python_unbuffered();
            let mut stdout = PythonText {
                inner: python_stdout(unbuffered),
                stderr: false,
                failed: false,
            };
            let mut stderr = PythonText {
                inner: std::io::stderr().lock(),
                stderr: true,
                failed: false,
            };
            let mut code = cli::main(&prog, &args, &cwd, &mut stdout, &mut stderr);
            // Interpreter shutdown flushed stdout, then stderr: output still
            // pending in either failed there with exit status 120.
            if let Err(error) = stdout.flush() {
                let _ = writeln!(
                    stderr,
                    "Exception ignored while flushing sys.stdout:\n{}",
                    oserror::stream_error(&error)
                );
                code = 120;
            }
            if stderr.failed && !unbuffered {
                code = 120;
            }
            code
        })
        .expect("the worker thread starts")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
