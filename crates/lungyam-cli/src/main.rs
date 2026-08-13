use std::{net::SocketAddr, path::PathBuf, sync::Arc};

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

    if let Some(warning) = public_admin_bind_warning(config.admin.enabled, &config.admin.listen) {
        eprintln!("{warning}");
    }

    let _admin_handle = if config.admin.enabled {
        Some(
            lungyam_admin::start_with_status_and_config_path(
                config.clone(),
                Arc::clone(&runtime_status),
                args.config.clone(),
            )
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

fn public_admin_bind_warning(enabled: bool, listen: &str) -> Option<String> {
    if !enabled {
        return None;
    }

    let address = listen.parse::<SocketAddr>().ok()?;
    if address.ip().is_loopback() {
        return None;
    }

    Some(format!(
        "warning: Lungyam admin is listening on non-loopback address {listen}; write actions remain disabled until authentication is configured"
    ))
}

#[cfg(test)]
mod tests {
    use super::public_admin_bind_warning;

    #[test]
    fn public_admin_bind_warns_only_when_enabled_and_non_loopback() {
        assert!(public_admin_bind_warning(false, "0.0.0.0:9090").is_none());
        assert!(public_admin_bind_warning(true, "127.0.0.1:9090").is_none());
        assert!(public_admin_bind_warning(true, "[::1]:9090").is_none());

        let ipv4 = public_admin_bind_warning(true, "0.0.0.0:9090").expect("public bind warning");
        assert!(ipv4.contains("non-loopback address 0.0.0.0:9090"));
        assert!(ipv4.contains("write actions remain disabled"));

        let ipv6 = public_admin_bind_warning(true, "[::]:9090").expect("public bind warning");
        assert!(ipv6.contains("non-loopback address [::]:9090"));
    }
}
