fn main() {
    let code = batch_git::run(std::env::args_os().collect());
    std::process::exit(code);
}
