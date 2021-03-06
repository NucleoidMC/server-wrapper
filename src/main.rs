#![feature(once_cell)]

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::FutureExt;
use tokio::fs;

pub use config::Config;
use executor::Executor;
use status::StatusWriter;

mod cache;
mod config;
mod executor;
mod status;
mod source;

const CACHE_ROOT: &str = "wrapper_cache";

const MIN_RESTART_INTERVAL: Duration = Duration::from_secs(4 * 60);

// TODO: implement triggers

#[derive(Clone)]
pub struct Context {
    pub github: source::github::Client,
    pub status: StatusWriter,
}

#[tokio::main]
pub async fn main() {
    loop {
        let config: Config = config::load("config.toml").await;
        let destinations: config::Destinations = config::load("destinations.toml").await;

        let status = match config.status.webhook {
            Some(webhook) => StatusWriter::from(status::webhook::Client::open(webhook)),
            None => StatusWriter::none(),
        };

        let github = source::github::Client::new(config.tokens.github.clone());
        let ctx = Context { github, status };

        let destinations: Vec<PreparedDestination> = prepare_destinations(&ctx, destinations.destinations).await;

        let changed_sources: Vec<_> = destinations.iter()
            .flat_map(|destination| destination.cache_files.iter())
            .filter(|(_, source)| source.changed())
            .map(|(name, _)| name.to_owned())
            .collect();

        for destination in destinations {
            destination.apply().await.expect("failed to apply destination");
        }

        let payload = if !changed_sources.is_empty() {
            let mut payload = status::Payload::new_sanitized(String::new());

            let description = format!(
                "Here's what changed:\n{}",
                changed_sources.into_iter()
                    .map(|source| format!(" - `{}`", source))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            payload.embeds.push(status::Embed {
                title: Some("Server starting up...".to_owned()),
                ty: status::EmbedType::Rich,
                description: Some(description),
                url: None,
                color: Some(0x00FF00),
            });

            payload
        } else {
            status::Payload::from("Starting up server...")
        };

        ctx.status.write(payload);

        let start = Instant::now();

        let mut executor = Executor::new(config.run);
        if let Err(err) = executor.run().await {
            eprintln!("server exited with error: {:?}", err);
        } else {
            println!("server closed");
        }

        let interval = Instant::now() - start;
        if interval < MIN_RESTART_INTERVAL {
            println!("server restarted very quickly! waiting a bit...");

            let delay = MIN_RESTART_INTERVAL - interval;
            ctx.status.write(format!("Server restarted too quickly! Waiting for {} seconds...", delay.as_secs()));

            tokio::time::sleep(delay.into()).await;
        } else {
            ctx.status.write("Server closed! Restarting...");
        }
    }
}

async fn prepare_destinations(ctx: &Context, destinations: HashMap<String, config::Destination>) -> Vec<PreparedDestination> {
    let mut futures = Vec::new();

    for (destination_name, destination) in destinations {
        let ctx = ctx.clone();
        let future = tokio::spawn(async move {
            prepare_destination(&ctx, &destination_name, &destination).await
                .expect(&format!("failed to prepare destination '{}'", destination_name))
        });
        futures.push(future.map(|result| result.unwrap()));
    }

    futures::future::join_all(futures).await
}

// TODO: load sources concurrently
async fn prepare_destination(ctx: &Context, destination_name: &str, destination: &config::Destination) -> Result<PreparedDestination> {
    let cache_root = Path::new(CACHE_ROOT).join(destination_name);

    let mut cache_files = Vec::with_capacity(destination.sources.len());

    let mut cache = cache::Loader::open(&cache_root).await?;
    for (_, source_set) in &destination.sources {
        for (key, source) in &source_set.sources {
            let cache_entry = cache.entry(key.clone());
            match source::load(ctx, cache_entry, source, &source_set.transform).await {
                Ok(reference) => cache_files.push((key.clone(), reference)),
                Err(err) => {
                    eprintln!("failed to load {}: {:?}! excluding.", key, err);
                    ctx.status.write(format!("Failed to load {}... Excluding!", key));
                }
            }
        }
    }

    cache.close().await?;

    Ok(PreparedDestination {
        root: destination.path.clone(),
        cache_files,
    })
}

struct PreparedDestination {
    root: PathBuf,
    cache_files: Vec<(String, cache::Reference)>,
}

impl PreparedDestination {
    async fn apply(&self) -> Result<()> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root).await?;
        }

        fs::create_dir_all(&self.root).await?;

        for (_, reference) in &self.cache_files {
            reference.copy_to(&self.root).await?;
        }

        Ok(())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error")]
    Io(#[from] io::Error),
    #[error("zip error")]
    Zip(#[from] zip::result::ZipError),
    #[error("http error")]
    Reqwest(#[from] reqwest::Error),
    #[error("malformed github reference")]
    MalformedGitHubReference(String),
    #[error("missing artifact")]
    MissingArtifact,
}
