use std::path::Path;

use crate::rustme::generate_in_directory;

pub mod rustme;

fn main() {
    if let Err(err) = generate_in_directory(Path::new(".")) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}
