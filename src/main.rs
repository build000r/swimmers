use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use swimmers::cli::{self, ConfigAction, ServerCli, ServerCommand};
use swimmers::config::Config;
use swimmers::{env_bootstrap, metrics, startup};

fn run_config_subcommand(action: Option<ConfigAction>) -> i32 {
    // Load .env so subcommands see the same environment the server would.
    let _ = dotenvy::dotenv();

    match action {
        None => {
            cli::print_config_table();
            0
        }
        Some(ConfigAction::Doctor) => {
            let config = Config::from_env();
            let tmux_present = cli::tmux_on_path();
            let data_dir = startup::resolve_data_dir();
            let data_dir_writable = cli::check_data_dir_writable(&data_dir);
            let findings = cli::run_doctor_checks(&config, tmux_present, data_dir_writable);
            cli::print_doctor_findings(&findings)
        }
    }
}

fn main() {
    let cli_args = ServerCli::parse();
    match cli_args.command {
        None | Some(ServerCommand::Serve) => {
            // Load .env before anything reads env vars.
            let _ = dotenvy::dotenv();
            env_bootstrap::bootstrap_provider_env_from_shell();

            // Initialize tracing.
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
                )
                .init();

            // Initialize Prometheus metrics exporter.
            let prom_handle = metrics::init_metrics();

            let config = Config::from_env();

            // Refuse to start if LocalTrust auth is paired with a non-loopback bind.
            // The pre-clap version only emitted a stderr warning here, which the
            // README's own external-access example silently relied on; that left
            // the API exposed to the network with no auth. Now we exit with
            // sysexits EX_CONFIG instead.
            if let Err(msg) = cli::enforce_localtrust_loopback(&config) {
                eprintln!("swimmers: {msg}");
                std::process::exit(cli::EXIT_CONFIG);
            }

            let config = Arc::new(config);
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("swimmers: failed to build tokio runtime: {err}");
                    std::process::exit(1);
                }
            };

            if let Err(err) = runtime.block_on(startup::run_server(config, prom_handle)) {
                tracing::error!("{err}");
                std::process::exit(1);
            }
        }
        Some(ServerCommand::Config { action }) => {
            std::process::exit(run_config_subcommand(action));
        }
    }
}
