fn main() {
    let args = std::env::args_os().skip(1).collect();
    std::process::exit(review_ready_gate::cli::command_thread(move || {
        review_ready_gate::cli::main(args)
    }));
}
