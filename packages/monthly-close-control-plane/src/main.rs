fn main() {
    let args = std::env::args_os().skip(1).collect();
    std::process::exit(monthly_close_control_plane::cli::command_thread(
        move || monthly_close_control_plane::cli::main(args),
    ));
}
