use std::path::Path;

use self::rustme::Configuration;

pub mod rustme;

fn main() -> Result<(), rustme::Error> {
    let config_path = config_path();
    let config_directory = config_path.parent().unwrap();
    let config = Configuration::load(config_path)?;
    config.generate(config_directory)?;

    Ok(())
}

fn config_path() -> &'static Path {
    let current_dir_path = Path::new(".rustme.ron");
    if current_dir_path.exists() {
        current_dir_path
    } else {
        Path::new(".rustme/config.ron")
    }
}
