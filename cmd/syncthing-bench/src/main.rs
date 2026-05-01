//! Syncthing sync benchmark tool
//!
//! Generates test datasets and verifies sync correctness via SHA-256 manifests.

use anyhow::{Context, Result};
use clap::Parser;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SyncBenchReport {
    scenario: String,
    source_dir: String,
    target_dir: String,
    files_total: usize,
    files_matched: usize,
    files_missing: Vec<String>,
    files_mismatch: Vec<String>,
    duration_ms: u64,
    success: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FileManifest {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Scenario {
    Small,
    Medium,
    Large,
    Mixed,
}

impl Scenario {
    fn generate(&self, root: &Path) -> Result<Vec<FileManifest>> {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root)?;

        let mut manifests = Vec::new();
        match self {
            Scenario::Small => {
                manifests.push(write_random_file(root, "test.bin", 4 * 1024)?);
            }
            Scenario::Medium => {
                manifests.push(write_random_file(root, "test.bin", 10 * 1024 * 1024)?);
            }
            Scenario::Large => {
                manifests.push(write_random_file(root, "test.bin", 1024 * 1024 * 1024)?);
            }
            Scenario::Mixed => {
                for i in 0..1000 {
                    let size = match i % 5 {
                        0 => 4 * 1024,
                        1 => 64 * 1024,
                        2 => 256 * 1024,
                        3 => 1024 * 1024,
                        _ => 10 * 1024 * 1024,
                    };
                    let subdir = root.join(format!("dir{}", i % 10));
                    std::fs::create_dir_all(&subdir)?;
                    manifests.push(write_random_file(&subdir, &format!("file{}.dat", i), size)?);
                }
            }
        }
        info!("Generated {} files for scenario {:?}", manifests.len(), self);
        Ok(manifests)
    }
}

fn write_random_file(dir: &Path, name: &str, size: usize) -> Result<FileManifest> {
    let path = dir.join(name);
    let mut data = vec![0u8; size];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut data);
    std::fs::write(&path, &data).with_context(|| format!("write {:?}", path))?;
    let hash = hex::encode(Sha256::digest(&data));
    Ok(FileManifest {
        path: path.to_string_lossy().to_string(),
        size: size as u64,
        sha256: hash,
    })
}

fn compute_manifest(dir: &Path) -> Result<Vec<FileManifest>> {
    let mut manifests = Vec::new();
    if !dir.exists() {
        return Ok(manifests);
    }
    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            let data = std::fs::read(path)
                .with_context(|| format!("read {:?}", path))?;
            let hash = hex::encode(Sha256::digest(&data));
            manifests.push(FileManifest {
                path: path.to_string_lossy().to_string(),
                size: data.len() as u64,
                sha256: hash,
            });
        }
    }
    Ok(manifests)
}

fn run(scenario: Scenario, source: &Path, target: &Path) -> Result<SyncBenchReport> {
    let start = std::time::Instant::now();

    let source_manifests = if source.exists() && std::fs::read_dir(source)?.next().is_some() {
        info!("Using existing source dataset at {:?}", source);
        compute_manifest(source)?
    } else {
        info!("Generating source dataset at {:?}", source);
        scenario.generate(source)?
    };

    let target_manifests = compute_manifest(target).unwrap_or_default();

    let source_map: HashMap<String, &FileManifest> = source_manifests
        .iter()
        .map(|m| {
            let rel = Path::new(&m.path)
                .strip_prefix(source)
                .unwrap_or(Path::new(&m.path))
                .to_string_lossy()
                .to_string();
            (rel, m)
        })
        .collect();

    let target_map: HashMap<String, &FileManifest> = target_manifests
        .iter()
        .map(|m| {
            let rel = Path::new(&m.path)
                .strip_prefix(target)
                .unwrap_or(Path::new(&m.path))
                .to_string_lossy()
                .to_string();
            (rel, m)
        })
        .collect();

    let mut files_matched = 0usize;
    let mut files_missing = Vec::new();
    let mut files_mismatch = Vec::new();

    for (rel, src) in &source_map {
        match target_map.get(rel) {
            Some(tgt) => {
                if src.sha256 == tgt.sha256 && src.size == tgt.size {
                    files_matched += 1;
                } else {
                    warn!("Mismatch: {} (src {} vs tgt {})", rel, src.sha256, tgt.sha256);
                    files_mismatch.push(rel.clone());
                }
            }
            None => {
                warn!("Missing in target: {}", rel);
                files_missing.push(rel.clone());
            }
        }
    }

    let success = files_missing.is_empty() && files_mismatch.is_empty() && files_matched == source_map.len();
    let report = SyncBenchReport {
        scenario: format!("{:?}", scenario),
        source_dir: source.to_string_lossy().to_string(),
        target_dir: target.to_string_lossy().to_string(),
        files_total: source_map.len(),
        files_matched,
        files_missing,
        files_mismatch,
        duration_ms: start.elapsed().as_millis() as u64,
        success,
    };

    let report_path = PathBuf::from("syncbench_report.json");
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, json)?;
    info!("Report written to {:?}", report_path);

    if success {
        info!("Syncbench PASSED: all {} files matched", files_matched);
    } else {
        warn!(
            "Syncbench FAILED: matched={}, missing={}, mismatch={}",
            files_matched,
            report.files_missing.len(),
            report.files_mismatch.len()
        );
    }

    Ok(report)
}

#[derive(Parser, Debug)]
#[command(name = "syncthing-bench")]
#[command(about = "Syncthing sync benchmark tool")]
struct Cli {
    /// Benchmark scenario
    #[arg(value_enum)]
    scenario: Scenario,

    /// Source directory (test data generated here if empty)
    #[arg(long)]
    source_dir: Option<PathBuf>,

    /// Target directory (verify sync result)
    #[arg(long)]
    target_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let source = cli.source_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("syncthing_bench_source_{}", std::process::id()))
    });
    let target = cli.target_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("syncthing_bench_target_{}", std::process::id()))
    });

    let report = run(cli.scenario, &source, &target)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
