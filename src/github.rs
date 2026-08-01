use std::{path::Path, process::Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::process::Command;

use crate::model::{Issue, PullRequest, Remote};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubConnectionState {
    CliMissing,
    Unauthenticated,
    NoRemote,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubVisibility {
    Private,
    Public,
}

#[derive(Debug, Clone)]
pub struct GitHub {
    root: std::path::PathBuf,
}

#[derive(Debug, Deserialize)]
struct GhAuthor {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    number: u64,
    title: String,
    state: String,
    author: GhAuthor,
    head_ref_name: String,
    base_ref_name: String,
    url: String,
    is_draft: bool,
}

#[derive(Debug, Deserialize)]
struct GhIssue {
    number: u64,
    title: String,
    state: String,
    author: GhAuthor,
    url: String,
}

impl GitHub {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }

    pub async fn connection_state(&self, remotes: &[Remote]) -> GitHubConnectionState {
        let installed = Command::new("gh")
            .arg("--version")
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success());
        let authenticated = installed
            && Command::new("gh")
                .args(["auth", "status"])
                .current_dir(&self.root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .is_ok_and(|status| status.success());
        classify_connection(installed, authenticated, remotes)
    }

    pub async fn publish_repository(
        &self,
        name: &str,
        visibility: GitHubVisibility,
        remote: &str,
        push: bool,
    ) -> Result<()> {
        let args = publish_arguments(name, visibility, remote, push);
        let output = Command::new("gh")
            .args(&args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .await
            .context("GitHub CLI is not installed")?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(())
    }

    pub async fn pull_requests(&self) -> Result<Vec<PullRequest>> {
        let output = self
            .run([
                "pr",
                "list",
                "--limit",
                "50",
                "--json",
                "number,title,state,author,headRefName,baseRefName,url,isDraft",
            ])
            .await?;
        let values: Vec<GhPullRequest> =
            serde_json::from_slice(&output).context("invalid response from gh pr list")?;
        Ok(values
            .into_iter()
            .map(|value| PullRequest {
                number: value.number,
                title: value.title,
                state: value.state,
                author: value.author.login,
                head: value.head_ref_name,
                base: value.base_ref_name,
                url: value.url,
                is_draft: value.is_draft,
            })
            .collect())
    }

    pub async fn issues(&self) -> Result<Vec<Issue>> {
        let output = self
            .run([
                "issue",
                "list",
                "--limit",
                "50",
                "--json",
                "number,title,state,author,url",
            ])
            .await?;
        let values: Vec<GhIssue> =
            serde_json::from_slice(&output).context("invalid response from gh issue list")?;
        Ok(values
            .into_iter()
            .map(|value| Issue {
                number: value.number,
                title: value.title,
                state: value.state,
                author: value.author.login,
                url: value.url,
            })
            .collect())
    }

    pub async fn pull_request_detail(&self, number: u64) -> Result<String> {
        let number = number.to_string();
        let output = self
            .run_vec(vec!["pr", "view", &number, "--comments"])
            .await?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    pub async fn issue_detail(&self, number: u64) -> Result<String> {
        let number = number.to_string();
        let output = self
            .run_vec(vec!["issue", "view", &number, "--comments"])
            .await?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    pub async fn checkout_pull_request(&self, number: u64) -> Result<()> {
        let number = number.to_string();
        self.run_vec(vec!["pr", "checkout", &number]).await?;
        Ok(())
    }

    pub async fn pull_request_checks(&self, number: u64) -> Result<String> {
        let number = number.to_string();
        let output = self
            .run_allow_failure(vec!["pr", "checks", &number])
            .await?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    pub async fn open_pull_request(&self, number: u64) -> Result<()> {
        let number = number.to_string();
        self.run_vec(vec!["pr", "view", &number, "--web"]).await?;
        Ok(())
    }

    pub async fn open_issue(&self, number: u64) -> Result<()> {
        let number = number.to_string();
        self.run_vec(vec!["issue", "view", &number, "--web"])
            .await?;
        Ok(())
    }

    async fn run<const N: usize>(&self, args: [&str; N]) -> Result<Vec<u8>> {
        self.run_vec(args.to_vec()).await
    }

    async fn run_vec(&self, args: Vec<&str>) -> Result<Vec<u8>> {
        let output = Command::new("gh")
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .await
            .context("GitHub CLI is not installed")?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(output.stdout)
    }

    async fn run_allow_failure(&self, args: Vec<&str>) -> Result<Vec<u8>> {
        let output = Command::new("gh")
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .await
            .context("GitHub CLI is not installed")?;
        if output.stdout.is_empty() && !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(output.stdout)
    }
}

fn classify_connection(
    installed: bool,
    authenticated: bool,
    remotes: &[Remote],
) -> GitHubConnectionState {
    if !installed {
        return GitHubConnectionState::CliMissing;
    }
    if !authenticated {
        return GitHubConnectionState::Unauthenticated;
    }
    if remotes.iter().any(|remote| {
        remote.fetch_url.contains("github.com") || remote.push_url.contains("github.com")
    }) {
        GitHubConnectionState::Ready
    } else {
        GitHubConnectionState::NoRemote
    }
}

fn publish_arguments(
    name: &str,
    visibility: GitHubVisibility,
    remote: &str,
    push: bool,
) -> Vec<String> {
    let mut args = vec![
        "repo".into(),
        "create".into(),
        name.into(),
        match visibility {
            GitHubVisibility::Private => "--private".into(),
            GitHubVisibility::Public => "--public".into(),
        },
        "--source=.".into(),
        "--remote".into(),
        remote.into(),
    ];
    if push {
        args.push("--push".into());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_authenticated_repositories_by_remote() {
        assert_eq!(
            classify_connection(false, false, &[]),
            GitHubConnectionState::CliMissing
        );
        assert_eq!(
            classify_connection(true, false, &[]),
            GitHubConnectionState::Unauthenticated
        );
        assert_eq!(
            classify_connection(true, true, &[]),
            GitHubConnectionState::NoRemote
        );
        let remote = Remote {
            name: "origin".into(),
            fetch_url: "git@github.com:me/repo.git".into(),
            push_url: "git@github.com:me/repo.git".into(),
        };
        assert_eq!(
            classify_connection(true, true, &[remote]),
            GitHubConnectionState::Ready
        );
    }

    #[test]
    fn builds_safe_publish_arguments() {
        assert_eq!(
            publish_arguments("demo", GitHubVisibility::Private, "origin", true),
            vec![
                "repo",
                "create",
                "demo",
                "--private",
                "--source=.",
                "--remote",
                "origin",
                "--push"
            ]
        );
        assert!(
            !publish_arguments("empty", GitHubVisibility::Public, "github", false)
                .contains(&"--push".into())
        );
    }
}
