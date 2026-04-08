// Copyright (c) 2024 Contributors to the image-rs project
//
// SPDX-License-Identifier: Apache-2.0
//
//! Standalone debug tool for testing image-rs image pull.
//!
//! Designed to be compiled as a static binary (musl) and deployed to
//! production environments for diagnosing image pull failures without
//! requiring the full Kata Containers stack.
//!
//! Build static binary (requires musl toolchain):
//!   cargo build --bin pull_debug --target x86_64-unknown-linux-musl \
//!     --no-default-features --features snapshot-overlayfs,oci-client-rustls
//!
//! Run:
//!   pull_debug --image docker.io/library/busybox:latest
//!   pull_debug --image myregistry.example.com/myimage:tag --auth user:pass
//!   RUST_LOG=debug pull_debug --image docker.io/library/busybox:latest

use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use image_rs::{
    config::{ImageConfig, ProxyConfig},
    registry::{Config as RegistryConfig, Registry},
};
use log::{error, info, warn};
use oci_client::Reference;

#[derive(Debug, Parser)]
#[command(
    name = "pull_debug",
    about = "Standalone image-rs pull debug tool for production environments",
    long_about = "Pull an OCI image using the image-rs library. \
                  Useful for standalone testing and debugging image pull issues \
                  in production environments without requiring the full \
                  Kata Containers stack.\n\n\
                  Set RUST_LOG=debug for verbose output."
)]
struct Args {
    /// OCI image reference to pull (e.g. docker.io/library/busybox:latest)
    #[arg(short, long)]
    image: String,

    /// Directory where the OCI bundle (rootfs + config.json) will be written.
    /// Defaults to <work-dir>/bundle.
    #[arg(short, long)]
    bundle_dir: Option<PathBuf>,

    /// Working directory for image-rs layer storage and metadata.
    #[arg(short, long, default_value = "/tmp/image-rs-debug")]
    work_dir: PathBuf,

    /// Registry credentials in `username:password` format for a private registry.
    /// For anonymous access, omit this flag.
    #[arg(short, long)]
    auth: Option<String>,

    /// Path to a Docker-format auth.json / config.json credentials file.
    /// When set, image-rs will use this file to authenticate to all registries.
    #[arg(long)]
    auth_file: Option<PathBuf>,

    /// Decrypt configuration for encrypted image layers.
    /// Format depends on the key provider (e.g. `<pem-path>:<password>`).
    #[arg(long)]
    decrypt_config: Option<String>,

    /// HTTPS proxy URL (e.g. http://proxy.example.com:3128)
    #[arg(long)]
    https_proxy: Option<String>,

    /// HTTP proxy URL
    #[arg(long)]
    http_proxy: Option<String>,

    /// Comma-separated list of hosts/CIDRs that bypass the proxy
    #[arg(long)]
    no_proxy: Option<String>,

    /// Allow pulling over plain HTTP from the given registry host.
    /// Can be specified multiple times (e.g. --insecure myregistry:5000).
    #[arg(long = "insecure", value_name = "HOST[:PORT]")]
    insecure_registries: Vec<String>,

    /// Maximum number of image layers to download concurrently
    #[arg(long, default_value = "3")]
    max_concurrent_downloads: usize,

    /// Path to a PEM file containing extra CA certificates to trust
    #[arg(long)]
    ca_cert: Option<PathBuf>,

    /// Skip verifying the unpacked bundle contents after pull
    #[arg(long)]
    skip_verify: bool,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    info!("=== image-rs pull debug tool ===");
    info!("Image:    {}", args.image);
    info!("Work dir: {}", args.work_dir.display());

    // --- Resolve and validate the image reference early so errors surface fast ---
    let reference = match Reference::try_from(args.image.as_str()) {
        Ok(r) => r,
        Err(e) => {
            error!("Invalid image reference {:?}: {e}", args.image);
            std::process::exit(1);
        }
    };
    info!(
        "Resolved: registry={} repository={} tag={:?} digest={:?}",
        reference.registry(),
        reference.repository(),
        reference.tag(),
        reference.digest(),
    );

    // --- Prepare directories ---
    if let Err(e) = std::fs::create_dir_all(&args.work_dir) {
        error!(
            "Failed to create work directory {}: {e}",
            args.work_dir.display()
        );
        std::process::exit(1);
    }

    let bundle_dir = args.bundle_dir.clone().unwrap_or(args.work_dir.join("bundle"));
    info!("Bundle:   {}", bundle_dir.display());
    if let Err(e) = std::fs::create_dir_all(&bundle_dir) {
        error!(
            "Failed to create bundle directory {}: {e}",
            bundle_dir.display()
        );
        std::process::exit(1);
    }

    // --- Build ImageConfig ---
    let mut config = ImageConfig::new(args.work_dir.clone());
    config.max_concurrent_layer_downloads_per_image = args.max_concurrent_downloads;

    // Proxy
    if args.https_proxy.is_some() || args.http_proxy.is_some() || args.no_proxy.is_some() {
        info!(
            "Proxy: https={:?} http={:?} no_proxy={:?}",
            args.https_proxy, args.http_proxy, args.no_proxy
        );
        config.image_pull_proxy = Some(ProxyConfig {
            https_proxy: args.https_proxy.clone(),
            http_proxy: args.http_proxy.clone(),
            no_proxy: args.no_proxy.clone(),
        });
    }

    // Extra CA certificates
    if let Some(ca_path) = &args.ca_cert {
        match std::fs::read_to_string(ca_path) {
            Ok(pem) => {
                info!("Loaded CA cert from {}", ca_path.display());
                config.extra_root_certificates.push(pem);
            }
            Err(e) => {
                error!("Failed to read CA cert {}: {e}", ca_path.display());
                std::process::exit(1);
            }
        }
    }

    // Insecure registries → registry_config
    if !args.insecure_registries.is_empty() {
        warn!(
            "Insecure (plain-HTTP) mode enabled for: {:?}",
            args.insecure_registries
        );
        let registries = args
            .insecure_registries
            .iter()
            .map(|host| Registry {
                prefix: host.clone(),
                insecure: true,
                blocked: false,
                location: host.clone(),
                mirror: vec![],
            })
            .collect();
        config.registry_config = Some(RegistryConfig {
            unqualified_search_registries: vec!["docker.io".into()],
            registry: registries,
        });
    }

    // Auth file → resource URI (read by the builder's resource provider)
    if let Some(auth_file) = &args.auth_file {
        let abs = match auth_file.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                error!("Auth file not found {}: {e}", auth_file.display());
                std::process::exit(1);
            }
        };
        info!("Auth file: {}", abs.display());
        config.authenticated_registry_credentials_uri =
            Some(format!("file://{}", abs.display()));
    }

    // --- Build the ImageClient ---
    info!("Building ImageClient...");
    let mut client = match image_rs::builder::ClientBuilder::from(config).build().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to build ImageClient: {e:#}");
            std::process::exit(1);
        }
    };

    // --auth flag is passed per-pull call as `username:password`
    let auth_info: Option<String> = args.auth.clone();

    // --- Pull ---
    info!("Pulling {}...", args.image);
    let start = Instant::now();

    let result = client
        .pull_image(
            &args.image,
            &bundle_dir,
            &auth_info.as_deref(),
            &args.decrypt_config.as_deref(),
        )
        .await;

    let elapsed = start.elapsed();

    match result {
        Ok(image_id) => {
            info!("Pull succeeded in {elapsed:.2?}");
            info!("Image ID: {image_id}");
            if !args.skip_verify {
                verify_bundle(&bundle_dir);
            }
            // Print the image ID to stdout so it can be captured by scripts
            println!("{image_id}");
        }
        Err(e) => {
            error!("Pull FAILED after {elapsed:.2?}");
            error!("{e:#}");
            std::process::exit(1);
        }
    }
}

/// Walk the bundle directory and print its contents as a sanity check.
fn verify_bundle(bundle_dir: &std::path::Path) {
    let config_json = bundle_dir.join("config.json");
    let rootfs = bundle_dir.join("rootfs");

    if config_json.exists() {
        info!("Bundle check: config.json PRESENT");
    } else {
        warn!("Bundle check: config.json MISSING");
    }

    if rootfs.exists() && rootfs.is_dir() {
        let entries = std::fs::read_dir(&rootfs)
            .map(|rd| rd.count())
            .unwrap_or(0);
        info!("Bundle check: rootfs/ PRESENT ({entries} top-level entries)");
    } else {
        warn!("Bundle check: rootfs/ MISSING or not a directory");
    }
}
