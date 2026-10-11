use grant_acquittal_workpapers::{cli, oserror, pytext};

/// A Python 3.15 text stream in UTF-8 mode: `\r\n` for each newline on
/// Windows, and lone surrogates written as stdout's surrogateescape (the
/// original byte) or stderr's backslashreplace (`\udcXX`) give them.
struct PythonText<W: std::io::Write> {
    inner: W,
    stderr: bool,
}

impl<W: std::io::Write> std::io::Write for PythonText<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Every write comes from a formatted &str, so it is whole UTF-8.
        let text = String::from_utf8_lossy(buf);
        let mut out = Vec::with_capacity(buf.len());
        for ch in text.chars() {
            match pytext::surrogate_of(ch) {
                Some(surrogate @ 0xDC80..=0xDCFF) if !self.stderr => out.push((surrogate - 0xDC00) as u8),
                Some(surrogate) => out.extend_from_slice(format!("\\u{surrogate:04x}").as_bytes()),
                None if ch == '\n' && cfg!(windows) => out.extend_from_slice(b"\r\n"),
                None => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
            }
        }
        self.inner.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn main() {
    // An unexpected failure exits 1, as an uncaught exception did in the Python script.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("grant-workpaper: internal error: {info}");
        std::process::exit(1);
    }));
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
            cli::main(
                &prog,
                &args,
                &cwd,
                &mut PythonText {
                    inner: std::io::stdout().lock(),
                    stderr: false,
                },
                &mut PythonText {
                    inner: std::io::stderr().lock(),
                    stderr: true,
                },
            )
        })
        .expect("the worker thread starts")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
