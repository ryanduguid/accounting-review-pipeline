use elizabeth_anne_alexander::cli;
use elizabeth_anne_alexander::util::{Bundle, Env};

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
    let code = cli::main(
        &args,
        &env,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
