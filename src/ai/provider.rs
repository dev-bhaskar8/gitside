//! Provider adapters for a standalone text request, following OpenCode's
//! Generate.text and Claude utility-call boundaries. Git context stays in ai.rs.

use std::{env, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, process::Command};

use super::{nonempty, shorten_subject};
use crate::config::{AgentProvider, AiMode, AiSettings, ApiProvider};

/// Host instructions and repository data travel separately wherever supported.
/// This mirrors OpenCode's utility system-prompt extraction for a fresh task.
pub(super) struct TextRequest {
    pub system: String,
    pub prompt: String,
}

impl TextRequest {
    fn combined_prompt(&self) -> String {
        format!("{}\n\n{}", self.system, self.prompt)
    }
}

pub(super) async fn generate(
    settings: &AiSettings,
    root: &Path,
    request: &TextRequest,
    api_key: Option<&str>,
) -> Result<String> {
    match settings.mode {
        AiMode::Agent => generate_with_agent(settings, root, request).await,
        AiMode::Api => generate_with_api(settings, request, api_key).await,
        AiMode::Local => bail!("offline rules do not use a text provider"),
    }
}

async fn generate_with_agent(
    settings: &AiSettings,
    root: &Path,
    request: &TextRequest,
) -> Result<String> {
    let prompt = request.combined_prompt();
    let (program, configured_args) = agent_command_parts(settings)?;
    let output_directory =
        tempfile::tempdir().context("failed to create agent output directory")?;
    let final_message = output_directory.path().join("message.txt");
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
            command.arg("--output-last-message").arg(&final_message);
            let system_file = output_directory.path().join("instructions.md");
            tokio::fs::write(&system_file, &request.system).await?;
            let path = toml::Value::String(system_file.to_string_lossy().into_owned());
            command.args(["--config", &format!("model_instructions_file={path}")]);
            command.args(["--config", "project_doc_max_bytes=0"]);
            command.args(["--config", "features.shell_tool=false"]);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            command.args(&settings.agent.args).arg("-");
        }
        AgentProvider::Claude => {
            isolate_claude(&mut command);
            command.args([
                "-p",
                "--permission-mode",
                "dontAsk",
                "--no-session-persistence",
                "--output-format",
                "json",
                "--tools",
                "",
                "--setting-sources",
                "",
                "--settings",
                "{\"disableAllHooks\":true}",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
            ]);
            // Like OpenCode's utility calls, replace the coding system prompt.
            command.arg("--system-prompt").arg(&request.system);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            command.args(&settings.agent.args);
        }
        AgentProvider::Opencode => {
            let inherited = env::var("OPENCODE_CONFIG_CONTENT").ok();
            command.env(
                "OPENCODE_CONFIG_CONTENT",
                opencode_utility_config(inherited.as_deref(), request)?,
            );
            command.args(["run", "--format", "json", "--agent", "gitside-commit"]);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            command.args(&settings.agent.args);
        }
        AgentProvider::Custom => {
            command.args(configured_args).args(&settings.agent.args);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;
    // Drain output while writing stdin: large prompts and verbose startup output
    // must not deadlock on full OS pipe buffers. Dropping stdin sends EOF.
    let mut stdin = child.stdin.take().context("agent input was unavailable")?;
    let input = async {
        let input = if settings.agent.provider == AgentProvider::Custom {
            &prompt
        } else {
            &request.prompt
        };
        let result = stdin.write_all(input.as_bytes()).await;
        drop(stdin);
        result
    };
    let (input_result, output) = tokio::join!(input, child.wait_with_output());
    let output = output?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if settings.agent.provider == AgentProvider::Opencode {
        // OpenCode can report provider errors as JSON even with an empty stderr.
        let parsed = parse_opencode_response(&stdout);
        if !output.status.success() {
            if let Err(error) = parsed {
                bail!("{program} failed: {error}");
            }
        } else {
            input_result.context("failed to send generation context to agent")?;
            return parsed;
        }
    }
    if settings.agent.provider == AgentProvider::Claude && !stdout.trim().is_empty() {
        let parsed = parse_claude_response(&stdout);
        if output.status.success() {
            input_result.context("failed to send generation context to agent")?;
            return parsed;
        }
        if let Err(error) = parsed {
            bail!("{program} failed: {error}");
        }
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        bail!("{program} failed: {}", shorten_subject(detail, 300));
    }
    input_result.context("failed to send generation context to agent")?;
    if settings.agent.provider == AgentProvider::Codex {
        return tokio::fs::read_to_string(&final_message)
            .await
            .context("Codex did not produce a final message; check its login and model");
    }
    Ok(stdout.into_owned())
}

// OpenCode's standalone utility pattern: a task system prompt, no tools,
// and a single step. Add it only to this child process's config, retaining
// the user's provider/model settings and existing authentication.
pub(super) fn opencode_utility_config(
    inherited: Option<&str>,
    request: &TextRequest,
) -> Result<String> {
    let mut config: Value = match inherited.filter(|content| !content.trim().is_empty()) {
        Some(content) => {
            jsonc_parser::parse_to_serde_value::<Option<Value>>(content, &Default::default())
                .map_err(|_| {
                    anyhow!("invalid OPENCODE_CONFIG_CONTENT; expected a JSON or JSONC object")
                })?
                .unwrap_or_else(|| json!({}))
        }
        None => json!({}),
    };
    let object = config
        .as_object_mut()
        .context("OPENCODE_CONFIG_CONTENT must be an object")?;
    // Native v2 agent entries take precedence over legacy entries during migration.
    if let Some(agents) = object.get_mut("agents").and_then(Value::as_object_mut) {
        agents.remove("gitside-commit");
    }
    let agents = object
        .entry("agent")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("OpenCode agent configuration must be an object")?;
    agents.insert(
        "gitside-commit".into(),
        json!({
            "description": "Generate a Git commit message from supplied changes",
            "mode": "primary",
            "prompt": request.system,
            "tools": {"*": false},
            "permission": "deny",
            "steps": 1
        }),
    );
    Ok(serde_json::to_string(&config)?)
}

// Adapted from OpenCode's ClaudeQueryOptions.ISOLATION_ENV and base options.
// See docs/third-party-notices.md for the upstream MIT notice and source.
fn isolate_claude(command: &mut Command) {
    command.envs([
        ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1"),
        ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
        ("DISABLE_AUTO_COMPACT", "1"),
        ("CLAUDE_CODE_ENABLE_AWAY_SUMMARY", "0"),
        ("ENABLE_TOOL_SEARCH", "false"),
        ("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1"),
    ]);
    // Simple/bare mode skips OAuth. Leave subscription authentication to Claude.
    command.env_remove("CLAUDE_CODE_SIMPLE");
    command.env_remove("CLAUDE_CODE_EFFORT_LEVEL");
    command.env_remove("MCP_TOOL_TIMEOUT");
}

pub(super) fn parse_claude_response(output: &str) -> Result<String> {
    let value: Value = serde_json::from_str(output).context("Claude Code returned invalid JSON")?;
    if value["is_error"] == true {
        let errors = value["errors"].as_array().map(|errors| {
            errors
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("; ")
        });
        bail!(
            "Claude Code: {}",
            shorten_subject(
                value["result"]
                    .as_str()
                    .or(errors.as_deref())
                    .unwrap_or("generation failed; check Claude Code authentication and model"),
                300
            )
        );
    }
    value["result"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .context("Claude Code returned no final message")
}

pub(super) fn parse_opencode_response(output: &str) -> Result<String> {
    let mut parts = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).context(
            "OpenCode returned invalid JSON; use a version supporting run --format json",
        )?;
        match event["type"].as_str() {
            Some("text") => {
                if let Some(text) = event["part"]["text"].as_str() {
                    parts.push(text.to_owned());
                }
            }
            Some("error") => {
                let message = event["error"]["message"]
                    .as_str()
                    .or_else(|| event["error"]["data"]["message"].as_str())
                    .unwrap_or("provider request failed; check OpenCode authentication and model");
                bail!("OpenCode: {}", shorten_subject(message, 300));
            }
            _ => {} // Reasoning, tools and progress are never commit text.
        }
    }
    let message = parts.join("\n");
    if message.trim().is_empty() {
        bail!("OpenCode returned no message; check its authentication and selected model");
    }
    Ok(message)
}

async fn generate_with_api(
    settings: &AiSettings,
    request: &TextRequest,
    supplied_key: Option<&str>,
) -> Result<String> {
    let model = nonempty(settings.api.model.as_deref()).context("set ai.api.model")?;
    let system = &request.system;
    let prompt = &request.prompt;
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
                    "system": system,
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
                .json(&json!({
                    "systemInstruction": {"parts": [{"text": system}]},
                    "contents": [{"parts": [{"text": prompt}]}]
                }))
                .send()
                .await?
        }
        ApiProvider::Openai | ApiProvider::Openrouter | ApiProvider::Compatible => {
            let mut request = client.post(endpoint).json(&json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt}
                ]
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
    parse_api_response(settings.api.provider, &redact_secret(body, key.as_deref()))
}

pub(super) fn redact_secret(value: String, secret: Option<&str>) -> String {
    secret
        .filter(|secret| !secret.is_empty())
        .map(|secret| value.replace(secret, "[REDACTED]"))
        .unwrap_or(value)
}

pub(super) fn parse_api_response(provider: ApiProvider, body: &str) -> Result<String> {
    let value: Value = serde_json::from_str(body).context("AI API returned invalid JSON")?;
    if let Some(error) = value.get("error") {
        bail!(
            "AI provider: {}",
            shorten_subject(error["message"].as_str().unwrap_or("request failed"), 300)
        );
    }
    let text_parts = |parts: &Value| -> String {
        parts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|part| part["thought"] != true)
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = match provider {
        ApiProvider::Anthropic => text_parts(&value["content"]),
        ApiProvider::Gemini => text_parts(&value["candidates"][0]["content"]["parts"]),
        ApiProvider::Openai | ApiProvider::Openrouter | ApiProvider::Compatible => {
            let message = &value["choices"][0]["message"];
            if let Some(refusal) = message["refusal"].as_str() {
                bail!(
                    "AI provider declined generation: {}",
                    shorten_subject(refusal, 300)
                );
            }
            message["content"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| text_parts(&message["content"]))
        }
    };
    if text.trim().is_empty() {
        bail!(
            "AI API response did not contain a commit message; check the model and provider limits"
        );
    }
    Ok(text)
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

pub(super) fn agent_program(settings: &AiSettings) -> String {
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

pub(super) fn agent_command_parts(settings: &AiSettings) -> Result<(String, Vec<String>)> {
    let command = agent_program(settings);
    if settings.agent.provider != AgentProvider::Custom {
        return Ok((command, Vec::new()));
    }
    let parsed = shell_words::split(&command).context("invalid custom agent command")?;
    let Some((program, args)) = parsed.split_first() else {
        bail!("custom agent command is empty");
    };
    Ok((program.clone(), args.to_vec()))
}

pub(super) fn executable_available(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return is_executable(path);
    }
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| {
            let candidate = directory.join(command);
            is_executable(&candidate)
                || (cfg!(windows) && directory.join(format!("{command}.exe")).is_file())
        })
    })
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
