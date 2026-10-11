use grant_acquittal_workpapers::cli;

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
    // An unexpected failure exits 1, as an uncaught exception did in the Python script.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("grant-workpaper: internal error: {info}");
        std::process::exit(1);
    }));
    let mut argv = std::env::args_os().map(|a| a.to_string_lossy().into_owned());
    // argparse names the program after the basename of argv[0].
    let prog = argv
        .next()
        .and_then(|first| {
            std::path::Path::new(&first)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "grant-workpaper".into());
    let args: Vec<String> = argv.collect();
    let cwd = std::env::current_dir().expect("the working directory must exist");
    // Deeply nested JSON recurses; give it the room CPython's own limit allows.
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            cli::main(
                &prog,
                &args,
                &cwd,
                &mut Newlines(std::io::stdout().lock()),
                &mut Newlines(std::io::stderr().lock()),
            )
        })
        .expect("the worker thread starts")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
