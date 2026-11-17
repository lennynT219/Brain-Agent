fn main() {
    if let Err(error) = brain::run_cli() {
        eprintln!("error:{error:#}");
        std::process::exit(1);
    }
}
