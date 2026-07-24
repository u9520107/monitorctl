fn main() {
    if let Err(error) = monitorctl_core::run_probe() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
