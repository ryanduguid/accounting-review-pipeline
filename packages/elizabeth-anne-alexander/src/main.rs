use elizabeth_anne_alexander::cli;
use elizabeth_anne_alexander::util::{Bundle, Env};

/// Python's text-mode stdout and stderr write `\r\n` for each newline on Windows.
struct Newlines<W: std::io::Write>(W);

impl<W: std::io::Write> std::io::Write for Newlines<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if cfg!(windows) {
            for (i, chunk) in buf.split(|&b| b == b'\n').enumerate() {
                if i > 0 {
                    self.0.write_all(b"\r\n")?;
                }
                self.0.write_all(chunk)?;
            }
            Ok(buf.len())
        } else {
            self.0.write(buf)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn main() {
    // An unexpected failure exits 1, as an uncaught exception did in the Python release.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("elizabeth-anne-alexander: internal error: {info}");
        std::process::exit(1);
    }));
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
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
                &mut Newlines(std::io::stdout().lock()),
                &mut Newlines(std::io::stderr().lock()),
            )
        })
        .expect("the worker thread starts")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
