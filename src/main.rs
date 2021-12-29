use crate::rustme::generate;

pub mod rustme;

fn main() {
    if let Err(err) = generate(std::env::args().nth(1).as_deref() == Some("--release")) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}
