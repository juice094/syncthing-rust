use clap::{Parser, Subcommand};
use obsidian_vault_crypto_adapter::{CryptoAdapter, Result};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "obsidian-vault-crypto-adapter")]
#[command(about = "Obsidian vault end-to-end encryption adapter for syncthing-rust")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new random salt and write it to a file
    GenSalt {
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Encrypt a plaintext vault into a sync folder
    Encrypt {
        #[arg(short, long)]
        vault: PathBuf,
        #[arg(short, long)]
        sync: PathBuf,
        #[arg(short, long)]
        password: String,
        #[arg(short, long)]
        salt_file: PathBuf,
    },
    /// Decrypt a sync folder back into a plaintext vault
    Decrypt {
        #[arg(short, long)]
        sync: PathBuf,
        #[arg(short, long)]
        vault: PathBuf,
        #[arg(short, long)]
        password: String,
        #[arg(short, long)]
        salt_file: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::GenSalt { output } => {
            let salt = obsidian_vault_crypto_adapter::generate_salt();
            std::fs::write(&output, salt)?;
            println!("salt written to {}", output.display());
        }
        Commands::Encrypt {
            vault,
            sync,
            password,
            salt_file,
        } => {
            let salt = std::fs::read(&salt_file)?;
            let adapter = CryptoAdapter::from_password(&password, &salt)?;
            adapter.encrypt_dir(&vault, &sync)?;
            println!("encrypted {} -> {}", vault.display(), sync.display());
        }
        Commands::Decrypt {
            sync,
            vault,
            password,
            salt_file,
        } => {
            let salt = std::fs::read(&salt_file)?;
            let adapter = CryptoAdapter::from_password(&password, &salt)?;
            adapter.decrypt_dir(&sync, &vault)?;
            println!("decrypted {} -> {}", sync.display(), vault.display());
        }
    }

    Ok(())
}
