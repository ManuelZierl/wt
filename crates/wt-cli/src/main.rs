fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "__wt-worker")
    {
        if let Err(error) = wt_core::worker::serve() {
            eprintln!("worker: {error:#}");
            std::process::exit(2);
        }
        return;
    }
    std::process::exit(wt_cli::run(std::env::args_os()));
}
