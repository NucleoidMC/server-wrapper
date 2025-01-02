use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::source;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Destinations {
    pub destinations: HashMap<String, Destination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Destination {
    pub path: PathBuf,
    pub triggers: Vec<String>,
    pub sources: HashMap<String, SourceSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSet {
    #[serde(default = "Default::default")]
    pub transform: Transform,
    #[serde(flatten)]
    pub sources: HashMap<String, Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Transform {
    Direct,
    Unzip { unzip: Vec<Pattern> },
}

impl Default for Transform {
    fn default() -> Self {
        Transform::Direct
    }
}

impl Transform {
    pub async fn apply(&self, file: source::File) -> io::Result<Option<source::File>> {
        match self {
            Transform::Direct => Ok(Some(file)),
            Transform::Unzip { unzip } => transform::unzip(file, &unzip).await,
        }
    }
}

mod transform {
    use std::io;
    use std::io::Read;

    use bytes::Bytes;
    use zip::ZipArchive;

    use super::*;

    // TODO: potentially support loading multiple files + directories
    pub async fn unzip(
        file: source::File,
        patterns: &[Pattern],
    ) -> io::Result<Option<source::File>> {
        let patterns: Vec<Pattern> = patterns.iter().cloned().collect();

        tokio::task::spawn_blocking(move || {
            let cursor = io::Cursor::new(file.bytes.as_ref());
            let mut zip = ZipArchive::new(cursor)?;

            let jar_names: Vec<String> = zip
                .file_names()
                .filter(|path| matches_all(path, &patterns))
                .map(|path| path.to_owned())
                .collect();

            for name in jar_names {
                let mut file = zip.by_name(&name)?;
                if file.is_file() {
                    let mut bytes = Vec::with_capacity(file.size() as usize);
                    file.read_to_end(&mut bytes)?;

                    let bytes = Bytes::from(bytes);
                    return Ok(Some(source::File { name, bytes }));
                }
            }

            Ok(None)
        })
        .await
        .unwrap()
    }

    fn matches_all(path: &str, patterns: &[Pattern]) -> bool {
        let mut include = patterns.iter().filter(|pattern| !pattern.exclude);
        let mut exclude = patterns.iter().filter(|pattern| pattern.exclude);

        include.all(|pattern| pattern.glob.matches(path))
            && !exclude.any(|pattern| pattern.glob.matches(path))
    }
}

#[derive(Debug, Clone)]
pub struct Pattern {
    pub glob: glob::Pattern,
    pub exclude: bool,
}

impl Serialize for Pattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.exclude {
            serializer.serialize_str(&format!("!{}", self.glob))
        } else {
            serializer.serialize_str(self.glob.as_str())
        }
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut pattern: &str = Deserialize::deserialize(deserializer)?;
        let mut exclude = false;
        if pattern.starts_with("!") {
            pattern = &pattern[1..];
            exclude = true;
        }

        match glob::Pattern::new(pattern) {
            Ok(glob) => Ok(Pattern { glob, exclude }),
            Err(err) => Err(D::Error::custom(err)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Source {
    GitHubArtifacts {
        github: String,
        workflow: Option<String>,
        branch: Option<String>,
        artifact: Option<String>,
    },
    Modrinth {
        project_id: String,
        game_version: Option<String>,
        loader: Option<String>,
    },
    Url {
        url: String,
    },
    Path {
        path: PathBuf,
    },
}

impl Default for Destinations {
    fn default() -> Self {
        let mut destinations = HashMap::new();
        destinations.insert("mods".to_owned(), Destination {
            path: PathBuf::from("mods"),
            triggers: vec!["startup".to_owned()],
            sources: {
                let mut sources = HashMap::new();
                sources.insert("fabric-api".to_owned(), Source::Url {
                    url: "https://github.com/FabricMC/fabric/releases/download/0.29.3%2B1.16/fabric-api-0.29.3+1.16.jar".to_owned()
                });

                let mut source_sets = HashMap::new();
                source_sets.insert("jars".to_owned(), SourceSet {
                    transform: Transform::Direct,
                    sources,
                });

                source_sets
            },
        });

        Destinations { destinations }
    }
}
