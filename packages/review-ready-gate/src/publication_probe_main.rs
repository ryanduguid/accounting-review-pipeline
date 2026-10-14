fn main() {
    std::process::exit(review_ready_gate::cli::command_thread(
        review_ready_gate::publication::process_probe,
    ));
}
