use std::path::PathBuf;

use clap::Parser;
use lungyam_core::config::Config;
use lungyam_proxy::{run, runtime_banner};

#[derive(Debug, Parser)]
#[command(name = "lungyam", version, about = "Edge-native API proxy")]
struct Args {
    /// Path to the Lungyam YAML configuration file.
    #[arg(short, long, default_value = "config/lungyam.yaml")]
    config: PathBuf,
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let config = Config::from_path(&args.config).unwrap_or_else(|error| {
        eprintln!("failed to start {}: {error}", runtime_banner());
        std::process::exit(2);
    });

    let _admin_handle = if config.admin.enabled {
        Some(
            lungyam_admin::start(config.clone()).unwrap_or_else(|error| {
                eprintln!("failed to start Lungyam admin: {error}");
                std::process::exit(2);
            }),
        )
    } else {
        None
    };

    run(config);
}
