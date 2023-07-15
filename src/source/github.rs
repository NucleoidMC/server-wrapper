use std::cmp;
use std::sync::Arc;

use serde::Deserialize;

use crate::{cache, config, source, Error, Result};

pub async fn load<'a>(
    client: &Client,
    cache: cache::Entry<'a>,
    owner: &str,
    repository: &str,
    filter: Filter,
    transform: &config::Transform,
) -> Result<cache::Reference> {
    let latest_artifact = get_latest_artifact(client, owner, repository, filter).await?;

    if let Some((id, url, name)) = latest_artifact {
        use cache::UpdateResult::*;
        match cache.try_update(cache::Token::ArtifactId(id)) {
            Mismatch(updater) => {
                let name = format!("{}.zip", name);

                let response = client.get(&url).await?;
                let bytes = response.bytes().await?;
                let file = source::File { name, bytes };

                if let Some(file) = transform.apply(file).await? {
                    Ok(updater.update(file).await?)
                } else {
                    Err(Error::MissingArtifact)
                }
            }
            Match(reference) => Ok(reference),
        }
    } else {
        cache.get_existing().ok_or(Error::MissingArtifact)
    }
}

async fn get_latest_artifact(
    client: &Client,
    owner: &str,
    repository: &str,
    filter: Filter,
) -> Result<Option<(usize, String, String)>> {
    // TODO: we're not handling pagination, which means we rely on results being ordered by newest!

    let mut workflow_runs = client
        .get_workflow_runs(owner, repository)
        .await?
        .workflow_runs;
    workflow_runs.sort_by_key(|run| cmp::Reverse(run.updated_at));

    let workflow_runs = workflow_runs
        .into_iter()
        .filter(|run| filter.test_workflow(&run.name))
        .filter(|run| filter.test_branch(&run.head_branch));

    for run in workflow_runs {
        let mut artifacts = match &run.artifacts_url {
            Some(_) => {
                client
                    .get_artifacts(owner, repository, &run)
                    .await?
                    .artifacts
            }
            None => continue,
        };
        artifacts.sort_by_key(|artifact| cmp::Reverse(artifact.updated_at));

        let artifacts = artifacts
            .into_iter()
            .filter(|artifact| filter.test_artifact(&artifact.name));

        for artifact in artifacts {
            // early-exit when we find an expired build: we know nothing older will still be around
            if artifact.expired {
                return Ok(None);
            }

            if let Some(url) = artifact.archive_download_url {
                return Ok(Some((artifact.id, url, artifact.name)));
            }
        }
    }

    Ok(None)
}

#[derive(Clone, Debug)]
pub struct Filter {
    pub workflow: Option<String>,
    pub branch: Option<String>,
    pub artifact: Option<String>,
}

impl Filter {
    #[inline]
    pub fn test_workflow(&self, workflow: &str) -> bool {
        self.workflow
            .as_ref()
            .map(|r| r == workflow)
            .unwrap_or(true)
    }

    #[inline]
    pub fn test_branch(&self, branch: &str) -> bool {
        self.branch.as_ref().map(|r| r == branch).unwrap_or(true)
    }

    #[inline]
    pub fn test_artifact(&self, artifact: &str) -> bool {
        self.artifact
            .as_ref()
            .map(|r| r == artifact)
            .unwrap_or(true)
    }
}

#[derive(Clone)]
pub struct Client {
    client: Arc<reqwest::Client>,
}

impl Client {
    const BASE_URL: &'static str = "https://api.github.com";

    pub fn new(token: Option<String>) -> Client {
        let mut default_headers = reqwest::header::HeaderMap::new();

        if let Some(token) = token {
            let authorization = format!("Bearer {}", token);
            default_headers.insert(
                reqwest::header::AUTHORIZATION,
                authorization.parse().unwrap(),
            );
        }

        let client = reqwest::Client::builder()
            .gzip(true)
            .user_agent("server-wrapper (https://github.com/NucleoidMC/server-wrapper)")
            .default_headers(default_headers)
            .build()
            .unwrap();

        Client {
            client: Arc::new(client),
        }
    }

    async fn get_workflow_runs(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<WorkflowRunsResponse> {
        // Github documents the exclude_pull_requests parameter, but it doesn't seem to have any effect,
        // so also use event=push to exclude runs with event=pull_request
        let url = format!(
            "{}/repos/{}/{}/actions/runs?event=push&exclude_pull_requests=true",
            Client::BASE_URL,
            owner,
            repository
        );
        let response = self.get(&url).await?;
        Ok(response.json().await?)
    }

    async fn get_artifacts(
        &self,
        owner: &str,
        repository: &str,
        run: &WorkflowRun,
    ) -> Result<ArtifactsResponse> {
        let url = format!(
            "{}/repos/{}/{}/actions/runs/{}/artifacts",
            Client::BASE_URL,
            owner,
            repository,
            run.id
        );
        let response = self.get(&url).await?;
        Ok(response.json().await?)
    }

    #[inline]
    pub async fn get(&self, url: &str) -> Result<reqwest::Response> {
        Ok(self.client.get(url).send().await?)
    }
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
struct WorkflowRunsResponse {
    total_count: usize,
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
struct WorkflowRun {
    id: usize,
    name: String,
    head_branch: String,
    workflow_id: usize,
    artifacts_url: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
struct ArtifactsResponse {
    total_count: usize,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
struct Artifact {
    id: usize,
    node_id: String,
    name: String,
    size_in_bytes: usize,
    url: String,
    archive_download_url: Option<String>,
    expired: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}
