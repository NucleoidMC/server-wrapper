use bytes::Bytes;

use crate::{Error, Result};
use crate::cache;
use crate::config::{self, Source};
use crate::Context;

pub mod github;
pub mod http;
pub mod modrinth;
pub mod path;

pub async fn load<'a>(
    ctx: &Context,
    cache: cache::Entry<'a>,
    source: &config::Source,
    transform: &config::Transform,
) -> Result<cache::Reference> {
    match source {
        Source::GitHubArtifacts {
            github,
            workflow,
            branch,
            artifact,
        } => match github.split("/").collect::<Vec<&str>>().as_slice() {
            [owner, repository] => {
                let filter = github::Filter {
                    workflow: workflow.clone(),
                    branch: branch.clone(),
                    artifact: artifact.clone(),
                };

                github::load(&ctx.github, cache, owner, repository, filter, transform).await
            }
            _ => Err(Error::MalformedGitHubReference(github.clone())),
        },
        Source::Modrinth {
            project_id,
            game_version,
            loader,
        } => modrinth::load(&ctx.modrinth, cache, project_id, game_version, loader, transform).await,
        Source::Url { url } => http::load(&ctx.client, cache, url, transform).await,
        Source::Path { path } => path::load(cache, path, transform).await,
    }
}

pub struct File {
    pub name: String,
    pub bytes: Bytes,
}
