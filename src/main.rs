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
mod source;
mod status;

const CACHE_ROOT: &str = "wrapper_cache";

// TODO: implement triggers

#[derive(Clone)]
pub struct Context {
    pub github: source::github::Client,
    pub modrinth: source::modrinth::Client,
    pub client: reqwest::Client,
    pub status: StatusWriter,
}

#[tokio::main]
pub async fn main() {
    let config_path = std::env::args().nth(1).unwrap_or_else(|| "config.toml".to_owned());
    let destinations_path = std::env::args().nth(2).unwrap_or_else(|| "destinations.toml".to_owned());

    loop {
        let mut config: Config = config::load(&config_path).await;
        let destinations: config::Destinations = config::load(&destinations_path).await;

        if config.run.is_none() && config.restart {
            eprintln!("warning: `restart = true` has no effect if `run` is not set");
            config.restart = false;
        }

        // make immutable just to be sure
        let config = config;

        let min_restart_interval = Duration::from_secs(config.min_restart_interval_seconds);

        let status = match config.status.webhook {
            Some(webhook) => StatusWriter::from(status::webhook::Client::open(webhook)),
            None => StatusWriter::none(),
        };

        let client = reqwest::Client::builder()
            .gzip(true)
            .user_agent("server-wrapper (https://github.com/NucleoidMC/server-wrapper)")
            .build()
            .unwrap();
        let github = source::github::Client::new(config.tokens.github.clone());
        let modrinth = source::modrinth::Client::new(client.clone());
        let ctx = Context {
            github,
            modrinth,
            client,
            status,
        };

        let destinations: Vec<PreparedDestination> =
            prepare_destinations(&ctx, destinations.destinations).await;

        let changed_sources: Vec<_> = destinations
            .iter()
            .flat_map(|destination| destination.cache_files.iter())
            .filter(|(_, source)| source.changed())
            .map(|(name, _)| name.to_owned())
            .collect();

        for destination in destinations {
            destination
                .apply()
                .await
                .expect("failed to apply destination");
        }

        let payload = if !changed_sources.is_empty() {
            let mut payload = status::Payload::new_sanitized(String::new());

            let description = format!(
                "Here's what changed:\n{}",
                changed_sources
                    .into_iter()
                    .map(|source| format!("- `{}`", source))
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

        if let Some(run) = config.run {
            let start = Instant::now();

            let mut executor = Executor::new(run);
            if let Err(err) = executor.run().await {
                eprintln!("server exited with error: {:?}", err);
            } else {
                println!("server closed");
            }

            let interval = Instant::now() - start;
            if config.restart && interval < min_restart_interval {
                println!("server restarted very quickly! waiting a bit...");

                let delay = min_restart_interval - interval;
                ctx.status.write(format!(
                    "Server restarted too quickly! Waiting for {} seconds...",
                    delay.as_secs()
                ));

                tokio::time::sleep(delay.into()).await;
            } else {
                ctx.status.write(if config.restart {
                    "Server closed! Restarting..."
                } else {
                    "Server closed!"
                });
            }
        }
    }
}

async fn prepare_destinations(
    ctx: &Context,
    destinations: HashMap<String, config::Destination>,
) -> Vec<PreparedDestination> {
    let mut futures = Vec::new();

    for (destination_name, destination) in destinations {
        let ctx = ctx.clone();
        let future = tokio::spawn(async move {
            prepare_destination(&ctx, &destination_name, &destination)
                .await
                .expect(&format!(
                    "failed to prepare destination '{}'",
                    destination_name
                ))
        });
        futures.push(future.map(|result| result.unwrap()));
    }

    futures::future::join_all(futures).await
}

// TODO: load sources concurrently
async fn prepare_destination(
    ctx: &Context,
    destination_name: &str,
    destination: &config::Destination,
) -> Result<PreparedDestination> {
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
                    ctx.status
                        .write(format!("Failed to load {}... Excluding!", key));
                }
            }
        }
    }

    let old_files = cache.close().await?;

    Ok(PreparedDestination {
        root: destination.path.clone(),
        cache_files,
        old_files,
    })
}

struct PreparedDestination {
    root: PathBuf,
    cache_files: Vec<(String, cache::Reference)>,
    old_files: Vec<cache::Reference>,
}

impl PreparedDestination {
    async fn apply(&self) -> Result<()> {
        if self.root.exists() {
            for reference in &self.old_files {
                reference.remove_from(&self.root).await?;
            }
        } else {
            fs::create_dir_all(&self.root).await?;
        }

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
