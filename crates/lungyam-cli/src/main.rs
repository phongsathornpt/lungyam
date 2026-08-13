use std::{path::PathBuf, sync::Arc};

use clap::Parser;
use lungyam_core::{config::Config, runtime::RuntimeStatus};
use lungyam_proxy::{run_with_status, runtime_banner};

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
    let runtime_status = Arc::new(RuntimeStatus::from_config(&config));

    let _admin_handle = if config.admin.enabled {
        Some(
            lungyam_admin::start_with_status(config.clone(), Arc::clone(&runtime_status))
                .unwrap_or_else(|error| {
                    eprintln!("failed to start Lungyam admin: {error}");
                    std::process::exit(2);
                }),
        )
    } else {
        None
    };

    run_with_status(config, runtime_status);
}
