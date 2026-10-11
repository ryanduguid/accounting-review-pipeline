use elizabeth_anne_alexander::util::{Bundle, Env};
use elizabeth_anne_alexander::{cli, pypath, pytext};

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
        for cp in pytext::code_points(&text) {
            match char::from_u32(cp) {
                None if (0xDC80..=0xDCFF).contains(&cp) && !self.stderr => out.push((cp - 0xDC00) as u8),
                None => out.extend_from_slice(format!("\\u{cp:04x}").as_bytes()),
                Some('\n') if cfg!(windows) => out.extend_from_slice(b"\r\n"),
                Some(ch) => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
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
    // An unexpected failure exits 1, as an uncaught exception did in the Python release.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("elizabeth-anne-alexander: internal error: {info}");
        std::process::exit(1);
    }));
    // sys.argv as Python decodes it: undecodable bytes or UTF-16 stay as surrogates.
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| pypath::decode_os(&a)).collect();
    let cwd = std::env::current_dir().expect("the working directory must exist");
    #[cfg(feature = "differential")]
    let bundle = match std::env::var_os("EAA_DIFFERENTIAL_BUNDLE") {
        Some(dir) => Bundle::from_dir(std::path::Path::new(&dir)),
        None => Bundle::shipped(),
    };
    #[cfg(not(feature = "differential"))]
    let bundle = Bundle::shipped();
    let env = Env { bundle, cwd };
    // A deeply nested JSON input recurses; give it the room CPython's own limit allows.
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            cli::main(
                &args,
                &env,
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
