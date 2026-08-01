use std::{env, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, process::Command};

use crate::{
    config::{AgentProvider, AiMode, AiSettings, ApiProvider},
    git::GitRepo,
    model::Commit,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Added,
    Deleted,
    Renamed,
    Other,
}

#[derive(Debug, Clone)]
struct FileChange {
    kind: FileKind,
    path: String,
}

#[derive(Debug, Clone)]
struct GenerationContext {
    files: Vec<FileChange>,
    additions: usize,
    deletions: usize,
    diff: String,
    truncated: bool,
    recent_subjects: Vec<String>,
}

pub async fn generate(
    settings: &AiSettings,
    repo: &GitRepo,
    recent: &[Commit],
    api_key: Option<&str>,
) -> Result<String> {
    if !settings.enabled {
        bail!("commit-message generation is disabled in configuration");
    }
    let context = GenerationContext::load(repo, recent, settings.max_diff_bytes).await?;
    let generated = match settings.mode {
        AiMode::Local => generate_local(&context, settings.max_files),
        AiMode::Agent => generate_with_agent(settings, repo.root(), &context).await?,
        AiMode::Api => generate_with_api(settings, &context, api_key).await?,
    };
    normalize_message(&generated, settings.emoji)
}

pub fn mode_label(settings: &AiSettings) -> &'static str {
    match settings.mode {
        AiMode::Local => "Smart Local",
        AiMode::Agent => "Existing Agent",
        AiMode::Api => "Direct API",
    }
}

pub fn provider_label(settings: &AiSettings) -> &'static str {
    match settings.mode {
        AiMode::Local => "Offline rules",
        AiMode::Agent => match settings.agent.provider {
            AgentProvider::Codex => "Codex",
            AgentProvider::Claude => "Claude Code",
            AgentProvider::Opencode => "OpenCode",
            AgentProvider::Custom => "Custom command",
        },
        AiMode::Api => match settings.api.provider {
            ApiProvider::Openai => "OpenAI",
            ApiProvider::Anthropic => "Anthropic",
            ApiProvider::Gemini => "Gemini",
            ApiProvider::Openrouter => "OpenRouter",
            ApiProvider::Compatible => "Compatible endpoint",
        },
    }
}

pub fn readiness(settings: &AiSettings) -> String {
    if !settings.enabled {
        return "Disabled in configuration".into();
    }
    match settings.mode {
        AiMode::Local => "Ready · offline".into(),
        AiMode::Agent => {
            let command = agent_program(settings);
            if executable_available(&command) {
                format!("Ready · {command} detected")
            } else {
                format!("Unavailable · {command} not found")
            }
        }
        AiMode::Api => {
            if settings.api.model.as_deref().is_none_or(str::is_empty) {
                return "Incomplete · set ai.api.model".into();
            }
            let key_env = api_key_env(settings);
            if key_env.is_none() || key_env.is_some_and(|name| env::var_os(name).is_some()) {
                "Ready · staged diff leaves this machine".into()
            } else {
                format!("Unavailable · set {}", key_env.unwrap_or_default())
            }
        }
    }
}

impl GenerationContext {
    async fn load(repo: &GitRepo, recent: &[Commit], max_diff_bytes: usize) -> Result<Self> {
        let root = repo.root();
        let names = git_output(root, &["diff", "--cached", "--name-status"]);
        let stats = git_output(root, &["diff", "--cached", "--numstat"]);
        let diff = git_output(root, &["diff", "--cached", "--no-ext-diff", "--no-color"]);
        let (names, stats, diff) = tokio::try_join!(names, stats, diff)?;
        if names.trim().is_empty() {
            bail!("stage at least one change before generating a commit message");
        }

        let files = names
            .lines()
            .filter_map(|line| {
                let fields = line.split('\t').collect::<Vec<_>>();
                let status = fields.first()?.chars().next()?;
                let path = fields.last()?.to_string();
                let kind = match status {
                    'A' => FileKind::Added,
                    'D' => FileKind::Deleted,
                    'R' => FileKind::Renamed,
                    _ => FileKind::Other,
                };
                Some(FileChange { kind, path })
            })
            .collect::<Vec<_>>();
        let (additions, deletions) = stats.lines().fold((0, 0), |(add, delete), line| {
            let mut fields = line.split('\t');
            let added = fields
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let deleted = fields
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            (add + added, delete + deleted)
        });
        let (diff, truncated) = truncate_utf8(diff, max_diff_bytes);
        Ok(Self {
            files,
            additions,
            deletions,
            diff,
            truncated,
            recent_subjects: recent
                .iter()
                .take(8)
                .map(|commit| commit.subject.clone())
                .collect(),
        })
    }

    fn prompt(&self, instructions: &str) -> String {
        let files = self
            .files
            .iter()
            .map(|file| format!("- {}", file.path))
            .collect::<Vec<_>>()
            .join("\n");
        let recent = if self.recent_subjects.is_empty() {
            "- No previous commits".into()
        } else {
            self.recent_subjects
                .iter()
                .map(|subject| format!("- {subject}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let extra = if instructions.trim().is_empty() {
            String::new()
        } else {
            format!("\nRepository instructions:\n{}\n", instructions.trim())
        };
        format!(
            "Generate one Git commit message for the staged changes below.\n\
             Match the recent commit style. Use an imperative subject, preferably under 72 characters.\n\
             Add a short body only when it explains why or important behavior.\n\
             Do not use Markdown fences, labels, commentary, or emoji. Return only the commit message.\n\
             {extra}\nChanged files:\n{files}\n\nStatistics: +{} -{}{}\n\nRecent subjects:\n{recent}\n\nStaged diff:\n{}",
            self.additions,
            self.deletions,
            if self.truncated {
                " · diff truncated"
            } else {
                ""
            },
            self.diff
        )
    }
}

fn generate_local(context: &GenerationContext, max_files: usize) -> String {
    let action = if context
        .files
        .iter()
        .all(|file| file.kind == FileKind::Added)
    {
        "Add"
    } else if context
        .files
        .iter()
        .all(|file| file.kind == FileKind::Deleted)
    {
        "Delete"
    } else if context
        .files
        .iter()
        .all(|file| file.kind == FileKind::Renamed)
    {
        "Rename"
    } else if context
        .files
        .iter()
        .all(|file| is_documentation(&file.path))
    {
        "Document"
    } else if context.files.iter().all(|file| is_test(&file.path)) {
        "Test"
    } else if looks_like_fix(&context.diff) {
        "Fix"
    } else if context.additions > 20 && context.deletions > 20 {
        "Refactor"
    } else {
        "Update"
    };
    let shown = max_files.max(1).min(context.files.len());
    let mut target = context
        .files
        .iter()
        .take(shown)
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if context.files.len() > shown {
        target.push_str(&format!(" +{} more", context.files.len() - shown));
    }
    shorten_subject(&format!("{action} {target}"), 72)
}

async fn generate_with_agent(
    settings: &AiSettings,
    root: &Path,
    context: &GenerationContext,
) -> Result<String> {
    let prompt = context.prompt(&settings.instructions);
    let program = agent_program(settings);
    let mut command = Command::new(&program);
    command.current_dir(root).kill_on_drop(true);
    match settings.agent.provider {
        AgentProvider::Codex => {
            command.args([
                "exec",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "-C",
            ]);
            command.arg(root);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            command.args(&settings.agent.args).arg("-");
        }
        AgentProvider::Claude => {
            command.args([
                "-p",
                "--permission-mode",
                "plan",
                "--no-session-persistence",
                "--output-format",
                "text",
            ]);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            command.args(&settings.agent.args);
        }
        AgentProvider::Opencode => {
            command.args(["run", "--pure"]);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            command.args(&settings.agent.args).arg(&prompt);
        }
        AgentProvider::Custom => {
            command.args(&settings.agent.args);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;
    if settings.agent.provider != AgentProvider::Opencode {
        child
            .stdin
            .take()
            .context("agent input was unavailable")?
            .write_all(prompt.as_bytes())
            .await?;
    }
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        bail!("{program} failed: {}", shorten_subject(error.trim(), 240));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn generate_with_api(
    settings: &AiSettings,
    context: &GenerationContext,
    supplied_key: Option<&str>,
) -> Result<String> {
    let model = nonempty(settings.api.model.as_deref()).context("set ai.api.model")?;
    let prompt = context.prompt(&settings.instructions);
    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent(format!("gitside/{}", env!("CARGO_PKG_VERSION")))
        .build()?;
    let endpoint = api_endpoint(settings, model)?;
    let key = supplied_key.map(str::to_owned).or_else(|| {
        api_key_env(settings).and_then(|name| env::var(name).ok().filter(|value| !value.is_empty()))
    });

    let response = match settings.api.provider {
        ApiProvider::Anthropic => {
            client
                .post(endpoint)
                .header(
                    "x-api-key",
                    key.as_deref().context("Anthropic API key is required")?,
                )
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": model,
                    "max_tokens": 512,
                    "messages": [{"role": "user", "content": prompt}]
                }))
                .send()
                .await?
        }
        ApiProvider::Gemini => {
            client
                .post(endpoint)
                .header(
                    "x-goog-api-key",
                    key.as_deref().context("Gemini API key is required")?,
                )
                .json(&json!({"contents": [{"parts": [{"text": prompt}]}]}))
                .send()
                .await?
        }
        ApiProvider::Openai | ApiProvider::Openrouter | ApiProvider::Compatible => {
            let mut request = client.post(endpoint).json(&json!({
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.2
            }));
            if let Some(key) = key.as_deref() {
                request = request.bearer_auth(key);
            }
            request.send().await?
        }
    };
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        let safe_body = redact_secret(body, key.as_deref());
        bail!(
            "AI API returned {status}: {}",
            shorten_subject(&safe_body, 300)
        );
    }
    parse_api_response(settings.api.provider, &body)
        .map(|message| redact_secret(message, key.as_deref()))
}

fn redact_secret(value: String, secret: Option<&str>) -> String {
    secret
        .filter(|secret| !secret.is_empty())
        .map(|secret| value.replace(secret, "[REDACTED]"))
        .unwrap_or(value)
}

fn parse_api_response(provider: ApiProvider, body: &str) -> Result<String> {
    let value: Value = serde_json::from_str(body).context("AI API returned invalid JSON")?;
    let text = match provider {
        ApiProvider::Anthropic => value["content"]
            .as_array()
            .and_then(|parts| parts.iter().find(|part| part["type"] == "text"))
            .and_then(|part| part["text"].as_str()),
        ApiProvider::Gemini => value["candidates"][0]["content"]["parts"][0]["text"].as_str(),
        ApiProvider::Openai | ApiProvider::Openrouter | ApiProvider::Compatible => {
            value["choices"][0]["message"]["content"].as_str()
        }
    };
    text.map(str::to_owned)
        .ok_or_else(|| anyhow!("AI API response did not contain a commit message"))
}

fn api_endpoint(settings: &AiSettings, model: &str) -> Result<String> {
    if let Some(endpoint) = nonempty(settings.api.endpoint.as_deref()) {
        return Ok(endpoint.to_owned());
    }
    Ok(match settings.api.provider {
        ApiProvider::Openai => "https://api.openai.com/v1/chat/completions".into(),
        ApiProvider::Anthropic => "https://api.anthropic.com/v1/messages".into(),
        ApiProvider::Gemini => format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
        ),
        ApiProvider::Openrouter => "https://openrouter.ai/api/v1/chat/completions".into(),
        ApiProvider::Compatible => bail!("set ai.api.endpoint for a compatible API"),
    })
}

pub(crate) fn api_key_env(settings: &AiSettings) -> Option<&str> {
    if let Some(value) = nonempty(settings.api.api_key_env.as_deref()) {
        return Some(value);
    }
    match settings.api.provider {
        ApiProvider::Openai => Some("OPENAI_API_KEY"),
        ApiProvider::Anthropic => Some("ANTHROPIC_API_KEY"),
        ApiProvider::Gemini => Some("GEMINI_API_KEY"),
        ApiProvider::Openrouter => Some("OPENROUTER_API_KEY"),
        ApiProvider::Compatible => None,
    }
}

fn agent_program(settings: &AiSettings) -> String {
    settings
        .agent
        .command
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| match settings.agent.provider {
            AgentProvider::Codex => "codex".into(),
            AgentProvider::Claude => "claude".into(),
            AgentProvider::Opencode => "opencode".into(),
            AgentProvider::Custom => "commit-message-generator".into(),
        })
}

fn executable_available(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| {
            let candidate = directory.join(command);
            candidate.is_file()
                || (cfg!(windows) && directory.join(format!("{command}.exe")).is_file())
        })
    })
}

fn normalize_message(value: &str, emoji: bool) -> Result<String> {
    let mut message = value.trim().to_owned();
    if message.starts_with("```") {
        message = message
            .lines()
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    message = message.trim().trim_matches('"').trim().to_owned();
    if let Some(stripped) = message.strip_prefix("🤖") {
        message = stripped.trim_start().to_owned();
    }
    if message.is_empty() {
        bail!("generator returned an empty commit message");
    }
    if message.len() > 4096 {
        message = truncate_utf8(message, 4096).0;
    }
    if emoji {
        message.insert_str(0, "🤖 ");
    }
    Ok(message)
}

async fn git_output(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .context("Git is required but was not found")?;
    if !output.status.success() {
        bail!(
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value, false);
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    (value, true)
}

fn shorten_subject(value: &str, max_chars: usize) -> String {
    let mut result = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        result = result.trim_end().to_owned();
        result.push('…');
    }
    result
}

fn is_documentation(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("docs/")
        || lower.ends_with(".md")
        || lower.ends_with(".rst")
        || lower.ends_with(".txt")
}

fn is_test(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/test")
        || lower.starts_with("test")
        || lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
}

fn looks_like_fix(diff: &str) -> bool {
    diff.lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .flat_map(|line| line[1..].split(|character: char| !character.is_ascii_alphanumeric()))
        .any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "fix" | "fixed" | "bug" | "error" | "issue"
            )
        })
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        process::Command as StdCommand,
        thread,
    };
    use tempfile::TempDir;

    fn context(files: &[(FileKind, &str)]) -> GenerationContext {
        GenerationContext {
            files: files
                .iter()
                .map(|(kind, path)| FileChange {
                    kind: *kind,
                    path: (*path).into(),
                })
                .collect(),
            additions: 10,
            deletions: 2,
            diff: "+change\n".into(),
            truncated: false,
            recent_subjects: vec!["Improve terminal layout".into()],
        }
    }

    #[test]
    fn local_generation_summarizes_status_and_file_count() {
        assert_eq!(
            generate_local(&context(&[(FileKind::Added, "src/ai.rs")]), 3),
            "Add src/ai.rs"
        );
        assert_eq!(
            generate_local(
                &context(&[
                    (FileKind::Other, "src/a.rs"),
                    (FileKind::Other, "src/b.rs"),
                    (FileKind::Other, "src/c.rs"),
                ]),
                2,
            ),
            "Update src/a.rs, src/b.rs +1 more"
        );

        let mut fix = context(&[(FileKind::Other, "src/parser.rs")]);
        fix.diff = "+// Fix invalid parser input.\n".into();
        assert_eq!(generate_local(&fix, 3), "Fix src/parser.rs");

        let mut refactor = context(&[(FileKind::Other, "src/app.rs")]);
        refactor.additions = 30;
        refactor.deletions = 25;
        assert_eq!(generate_local(&refactor, 3), "Refactor src/app.rs");
    }

    #[test]
    fn emoji_is_applied_once_and_is_optional() {
        assert_eq!(
            normalize_message("Update UI", true).unwrap(),
            "🤖 Update UI"
        );
        assert_eq!(
            normalize_message("🤖 Update UI", false).unwrap(),
            "Update UI"
        );
    }

    #[test]
    fn api_errors_redact_the_active_credential() {
        assert_eq!(
            redact_secret("request rejected for sk-private".into(), Some("sk-private")),
            "request rejected for [REDACTED]"
        );
    }

    #[test]
    fn parses_supported_api_response_shapes() {
        assert_eq!(
            parse_api_response(
                ApiProvider::Openai,
                r#"{"choices":[{"message":{"content":"Update UI"}}]}"#,
            )
            .unwrap(),
            "Update UI"
        );
        assert_eq!(
            parse_api_response(
                ApiProvider::Anthropic,
                r#"{"content":[{"type":"text","text":"Update UI"}]}"#,
            )
            .unwrap(),
            "Update UI"
        );
        assert_eq!(
            parse_api_response(
                ApiProvider::Gemini,
                r#"{"candidates":[{"content":{"parts":[{"text":"Update UI"}]}}]}"#,
            )
            .unwrap(),
            "Update UI"
        );
    }

    async fn staged_repository() -> (TempDir, GitRepo) {
        let directory = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Gitside Test"],
            vec!["config", "user.email", "gitside@example.invalid"],
        ] {
            assert!(
                StdCommand::new("git")
                    .args(args)
                    .current_dir(directory.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join("src/ai.rs"), "pub fn generate() {}\n").unwrap();
        assert!(
            StdCommand::new("git")
                .args(["add", "src/ai.rs"])
                .current_dir(directory.path())
                .status()
                .unwrap()
                .success()
        );
        let repo = GitRepo::discover(directory.path()).await.unwrap();
        (directory, repo)
    }

    #[tokio::test]
    async fn local_mode_generates_from_the_real_staged_index() {
        let (_directory, repo) = staged_repository().await;
        let settings = AiSettings {
            enabled: true,
            emoji: true,
            ..AiSettings::default()
        };

        assert_eq!(
            generate(&settings, &repo, &[], None).await.unwrap(),
            "🤖 Add src/ai.rs"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn custom_agent_receives_context_and_returns_a_draft() {
        let (directory, repo) = staged_repository().await;
        let script = directory.path().join("generator.sh");
        fs::write(
            &script,
            "#!/bin/sh\ninput=$(cat)\nprintf 'Update AI adapter'\nprintf '%s' \"$input\" | grep -q 'src/ai.rs'\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let mut settings = AiSettings {
            enabled: true,
            mode: AiMode::Agent,
            ..AiSettings::default()
        };
        settings.agent.provider = AgentProvider::Custom;
        settings.agent.command = Some(script.to_string_lossy().into_owned());

        assert_eq!(
            generate(&settings, &repo, &[], None).await.unwrap(),
            "Update AI adapter"
        );
    }

    #[tokio::test]
    async fn compatible_api_uses_the_configured_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + length {
                    break;
                }
            }
            assert!(String::from_utf8_lossy(&request).contains("src/ai.rs"));
            let body = r#"{"choices":[{"message":{"content":"Add direct API mode"}}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let (_directory, repo) = staged_repository().await;
        let mut settings = AiSettings {
            enabled: true,
            mode: AiMode::Api,
            ..AiSettings::default()
        };
        settings.api.provider = ApiProvider::Compatible;
        settings.api.model = Some("test-model".into());
        settings.api.endpoint = Some(format!("http://{address}/chat/completions"));

        assert_eq!(
            generate(&settings, &repo, &[], None).await.unwrap(),
            "Add direct API mode"
        );
        server.join().unwrap();
    }
}
