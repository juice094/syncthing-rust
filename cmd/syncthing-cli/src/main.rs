//! Syncthing CLI utilities
//!
//! Provides generate-cert, show-id, and metrics-flush commands.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, warn};

use syncthing_net::SyncthingTlsConfig;

/// Syncthing CLI
#[derive(Parser, Debug)]
#[command(name = "syncthing-cli")]
#[command(about = "Syncthing command-line utilities")]
struct Cli {
    /// Configuration directory
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// Subcommand
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a new device certificate
    GenerateCert {
        /// Device name
        #[arg(short, long, default_value = "syncthing-rust")]
        device_name: String,

        /// Force overwrite existing certificate
        #[arg(short, long)]
        force: bool,
    },

    /// Show device ID
    ShowId,

    /// Flush collected metrics to CSV
    MetricsFlush {
        /// Output CSV path
        #[arg(default_value = "syncthing_metrics.csv")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let cli = Cli::parse();
    let config_dir = cli
        .config_dir
        .unwrap_or_else(syncthing_core::paths::default_config_dir);

    match cli.command {
        Commands::GenerateCert { device_name, force } => {
            cmd_generate_cert(&config_dir, &device_name, force).await?;
        }
        Commands::ShowId => {
            cmd_show_id(&config_dir).await?;
        }
        Commands::MetricsFlush { output } => {
            syncthing_net::metrics::global().flush_to_csv(&output)?;
            println!("Metrics flushed to {:?}", output);
        }
    }

    Ok(())
}

async fn cmd_generate_cert(config_dir: &PathBuf, device_name: &str, force: bool) -> Result<()> {
    info!("Generating new device certificate...");
    info!("Config directory: {:?}", config_dir);
    info!("Device name: {}", device_name);

    if !config_dir.exists() {
        tokio::fs::create_dir_all(config_dir).await?;
    }

    let cert_path = config_dir.join(syncthing_net::tls::CERT_FILE_NAME);
    let key_path = config_dir.join(syncthing_net::tls::KEY_FILE_NAME);

    if cert_path.exists() || key_path.exists() {
        if force {
            warn!("Existing certificates will be overwritten");
        } else {
            anyhow::bail!(
                "Certificates already exist. Use --force to overwrite, or use 'show-id' command to view the current device ID"
            );
        }
    }

    if cert_path.exists() {
        tokio::fs::remove_file(&cert_path).await?;
    }
    if key_path.exists() {
        tokio::fs::remove_file(&key_path).await?;
    }

    let tls_config = SyncthingTlsConfig::load_or_generate(config_dir)
        .await
        .context("failed to generate certificate")?;

    let device_id = tls_config.device_id();

    println!("Certificate generated successfully!");
    println!();
    println!("Device ID: {}", device_id);
    println!("Certificate path: {:?}", cert_path);
    println!("Private key path: {:?}", key_path);
    println!();
    println!("Please keep your private key file safe!");

    Ok(())
}

async fn cmd_show_id(config_dir: &std::path::Path) -> Result<()> {
    let cert_path = config_dir.join(syncthing_net::tls::CERT_FILE_NAME);
    let key_path = config_dir.join(syncthing_net::tls::KEY_FILE_NAME);

    if !cert_path.exists() || !key_path.exists() {
        println!("Certificate files not found. Please run 'generate-cert' first.");
        println!();
        println!("Expected paths:");
        println!("  Certificate: {:?}", cert_path);
        println!("  Private key: {:?}", key_path);
        return Ok(());
    }

    let tls_config = SyncthingTlsConfig::load_or_generate(config_dir)
        .await
        .context("failed to load certificate")?;

    let device_id = tls_config.device_id();

    println!("Device ID: {}", device_id);
    println!("Short ID:  {}", device_id.short_id());
    println!();
    println!("Certificate path: {:?}", cert_path);
    println!("Private key path: {:?}", key_path);

    Ok(())
}
