use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub use destinations::*;

mod destinations;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub run: Option<Vec<String>>,
    #[serde(default = "Default::default")]
    pub status: Status,
    #[serde(default = "Default::default")]
    pub tokens: Tokens,
    pub triggers: HashMap<String, Trigger>,
    #[serde(default = "default_min_restart_interval")]
    pub min_restart_interval_seconds: u64,
    #[serde()]
    pub restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Status {
    pub webhook: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tokens {
    pub github: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Trigger {
    #[serde(rename = "startup")]
    Startup,
    #[serde(rename = "webhook")]
    Webhook { port: u16 },
}

impl Default for Config {
    fn default() -> Self {
        Config {
            run: Some(vec!["java -jar fabric-server-launch.jar".to_owned()]),
            tokens: Tokens::default(),
            status: Status::default(),
            triggers: {
                let mut triggers = HashMap::new();
                triggers.insert("startup".to_owned(), Trigger::Startup);
                triggers
            },
            min_restart_interval_seconds: default_min_restart_interval(),
            restart: default_restart(),
        }
    }
}

fn default_min_restart_interval() -> u64 {
    240
}

fn default_restart() -> bool {
    true
}

pub async fn load<P, T>(path: P) -> T
where
    P: AsRef<Path>,
    T: Serialize + DeserializeOwned + Default,
{
    let path = path.as_ref();
    let mut config = if path.exists() {
        read_config(path).await.expect("failed to read config")
    } else {
        let config = T::default();
        write_config(path, &config)
            .await
            .expect("failed to write default config");
        config
    };

    config
}

async fn write_config<T: Serialize>(path: &Path, config: &T) -> io::Result<()> {
    let string = toml::to_string(config).expect("malformed config");

    let mut file = File::create(path).await?;
    file.write_all(string.as_bytes()).await?;

    Ok(())
}

async fn read_config<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    let mut file = File::open(path).await?;

    let mut string = String::new();
    file.read_to_string(&mut string).await?;

    Ok(toml::from_str::<T>(&string).expect("malformed config"))
}
