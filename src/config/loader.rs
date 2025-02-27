use std::{path::PathBuf, str::FromStr};

use reqwest::{Client, Url};

use super::Destinations;

pub enum DestinationsLoader {
    File(PathBuf),
    Url { client: Client, url: Url },
}

impl DestinationsLoader {
    pub fn from_path(path: &str) -> Self {
        Self::File(PathBuf::from_str(path).unwrap())
    }

    pub fn from_url(url: Url, client: Client) -> Self {
        Self::Url { client, url }
    }

    pub async fn load(&self) -> Destinations {
        match self {
            DestinationsLoader::File(path) => super::load(path).await,
            DestinationsLoader::Url { client, url } => {
                eprintln!("loading destinations from `{url}`");
                let content = client
                    .get(url.clone())
                    .send()
                    .await
                    .expect("failed to send request")
                    .error_for_status()
                    .expect("server returned error")
                    .text()
                    .await
                    .expect("failed to read request");
                // TODO: better error handling
                toml::from_str(&content).expect("invalid config")
            }
        }
    }
}
