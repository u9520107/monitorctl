fn main() {
    if let Err(error) = monitorctl_core::run_cli() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
