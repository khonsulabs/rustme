use walkdir::WalkDir;

use self::rustme::Configuration;

pub mod rustme;

fn main() -> Result<(), rustme::Error> {
    for entry in WalkDir::new(".").into_iter().filter_map(Result::ok) {
        let config_path = if entry.file_name() == ".rustme.ron" {
            entry.into_path()
        } else if entry.file_type().is_dir() && entry.file_name() == ".rustme" {
            entry.path().join("config.ron")
        } else {
            continue;
        };

        println!("Processing {:?}", config_path);
        let config = Configuration::load(config_path)?;
        config.generate(std::env::args().nth(1).as_deref() == Some("--release"))?;
    }

    Ok(())
}
