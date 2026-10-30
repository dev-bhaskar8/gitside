pub(crate) mod acp;
pub(crate) mod antigravity_install;
mod rpc;

// Provider adapters for T3-style structured commit generation.
// Shared prompt/decoding lives in ai/commit.rs; Git context stays in ai.rs.

use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

use super::{commit, nonempty, shorten_subject};
use crate::config::{AgentProvider, AiMode, AiSettings, ApiProvider};

/// Host instructions and repository data travel separately wherever supported.
/// This mirrors OpenCode's utility system-prompt extraction for a fresh task.
pub(super) struct TextRequest {
    pub system: String,
    pub prompt: String,
}

impl TextRequest {
    pub(super) fn combined_prompt(&self) -> String {
        format!("{}\n\n{}", self.system, self.prompt)
    }
}

#[cfg(test)]
pub(super) fn spawn_fixture_command(
    command: &mut Command,
) -> std::io::Result<tokio::process::Child> {
    // Parallel tests can fork while another fixture is being written. Its writer
    // descriptor briefly survives in that child until exec closes CLOEXEC fds,
    // and Linux rejects execution of the new fixture with ETXTBSY meanwhile.
    #[cfg(unix)]
    {
        let mut retries = 0;
        loop {
            match command.spawn() {
                Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && retries < 20 => {
                    retries += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }
    #[cfg(not(unix))]
    command.spawn()
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
    match settings.agent.provider {
        AgentProvider::Antigravity
        | AgentProvider::Cursor
        | AgentProvider::Grok
        | AgentProvider::Acp => return acp::generate(settings, request).await,
        AgentProvider::Pi | AgentProvider::Muse => return rpc::generate(settings, request).await,
        _ => {}
    }
    let prompt = request.combined_prompt();
    let (program, configured_args) = agent_command_parts(settings)?;
    let output_directory =
        tempfile::tempdir().context("failed to create agent output directory")?;
    let final_message = output_directory.path().join("message.txt");
    let schema_file = output_directory.path().join("schema.json");
    let schema = commit::schema().to_string();
    tokio::fs::write(&schema_file, &schema).await?;
    let mut command = Command::new(&program);
    command.current_dir(root).kill_on_drop(true);
    match settings.agent.provider {
        AgentProvider::Codex => {
            command.arg("exec").args(&settings.agent.args);
            command.args([
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "-C",
            ]);
            command.arg(root);
            command.arg("--output-last-message").arg(&final_message);
            command.arg("--output-schema").arg(&schema_file);
            command.args([
                "--config",
                "features.plugins=false",
                "--config",
                "features.apps=false",
            ]);
            let system_file = output_directory.path().join("instructions.md");
            tokio::fs::write(&system_file, &request.system).await?;
            let path = toml::Value::String(system_file.to_string_lossy().into_owned());
            command.args(["--config", &format!("model_instructions_file={path}")]);
            command.args(["--config", "project_doc_max_bytes=0"]);
            command.args(["--config", "features.shell_tool=false"]);
            if let Some(model) = nonempty(settings.agent.model.as_deref()) {
                command.args(["--model", model]);
            }
            if let Some(overrides) =
                codex_mcp_override(&program, root, &settings.agent.args).await?
            {
                command.args(["--config", &overrides]);
            }
            command.arg("-");
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
            command.args(["--json-schema", &schema, "--disable-slash-commands"]);
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
        AgentProvider::Antigravity
        | AgentProvider::Cursor
        | AgentProvider::Grok
        | AgentProvider::Acp
        | AgentProvider::Pi
        | AgentProvider::Muse => unreachable!("Protocol requests use their dedicated adapters"),
        AgentProvider::Custom => {
            command.args(configured_args).args(&settings.agent.args);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(test)]
    let spawned = spawn_fixture_command(&mut command);
    #[cfg(not(test))]
    let spawned = command.spawn();
    let mut child = spawned.with_context(|| format!("failed to start {program}"))?;
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
    let stdout = child
        .stdout
        .take()
        .context("agent output was unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("agent diagnostics were unavailable")?;
    let output = async {
        let (stdout, stderr, status) = tokio::try_join!(
            read_capped(stdout, 1_048_576),
            read_capped(stderr, 1_048_576),
            child.wait()
        )?;
        Ok::<_, anyhow::Error>(std::process::Output {
            status,
            stdout,
            stderr,
        })
    };
    let (input_result, output) =
        tokio::try_join!(async { Ok::<_, anyhow::Error>(input.await) }, output)?;
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
        let file = tokio::fs::File::open(&final_message)
            .await
            .context("Codex did not produce a final message; check its login and model")?;
        return String::from_utf8(read_capped(file, 128_000).await?)
            .context("Codex returned non-UTF-8 output");
    }
    Ok(stdout.into_owned())
}

async fn read_capped(reader: impl AsyncRead + Unpin, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > limit {
        return Err(std::io::Error::other(
            "AI provider exceeded the output limit",
        ));
    }
    Ok(bytes)
}

// Port of T3's named-MCP disabling: an empty table merges rather than clears.
async fn codex_mcp_override(program: &str, root: &Path, args: &[String]) -> Result<Option<String>> {
    let mut configs = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if matches!(arg.as_str(), "-c" | "--config") {
            if let Some(value) = iter.next() {
                configs.extend([arg.clone(), value.clone()]);
            }
        } else if arg.starts_with("--config=") {
            configs.push(arg.clone());
        }
    }
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(program)
            .args(configs)
            .args([
                "--config",
                "features.plugins=false",
                "--config",
                "features.apps=false",
                "mcp",
                "list",
                "--json",
            ])
            .current_dir(root)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Codex MCP configuration check timed out")??;
    if !output.status.success() {
        bail!("Could not inspect Codex MCP configuration; check the CLI version and configuration");
    }
    let servers: Value =
        serde_json::from_slice(&output.stdout).context("Codex MCP list returned invalid JSON")?;
    let servers = servers
        .as_array()
        .context("Codex MCP list did not return a server list")?;
    let mut disabled = Vec::new();
    for server in servers {
        if server["enabled"] == true {
            let name = server["name"]
                .as_str()
                .context("Codex MCP server had no name")?;
            disabled.push(format!(
                "{}={{enabled=false}}",
                toml::Value::String(name.to_owned())
            ));
        }
    }
    Ok((!disabled.is_empty()).then(|| format!("mcp_servers={{{}}}", disabled.join(","))))
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
    let parsed: Value =
        serde_json::from_str(output).context("Claude Code returned invalid JSON")?;
    // T3 accepts both normal envelopes and verbose message arrays.
    let value = if let Some(messages) = parsed.as_array() {
        messages
            .iter()
            .rev()
            .find(|message| message["type"] == "result")
            .context("Claude Code returned no result event")?
    } else {
        &parsed
    };
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
    if let Some(structured) = value
        .get("structured_output")
        .filter(|value| !value.is_null())
    {
        return Ok(structured.to_string());
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
            let mut body = json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt}
                ]
            });
            if settings.api.provider == ApiProvider::Openai {
                body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"commit_message","strict":true,"schema":commit::schema()}});
            }
            let mut request = client.post(endpoint).json(&body);
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
            AgentProvider::Cursor => "cursor-agent".into(),
            AgentProvider::Grok => "grok".into(),
            AgentProvider::Pi => "pi".into(),
            AgentProvider::Muse => "muse".into(),
            AgentProvider::Acp => "acp-agent".into(),
            AgentProvider::Antigravity => {
                if let Some(path) = antigravity_install::installed() {
                    return path.to_string_lossy().into_owned();
                }
                if cfg!(windows) {
                    "agy_acp_server.exe".into()
                } else {
                    "agy_acp_server.par".into()
                }
            }
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
    resolve_program(command).is_some()
}

pub(super) fn resolve_program(command: &str) -> Option<PathBuf> {
    fn candidate(path: &Path) -> Option<PathBuf> {
        if is_executable(path) {
            return std::fs::canonicalize(path).ok();
        }
        // npm-installed Pi and other coding CLIs commonly use Windows launchers.
        if cfg!(windows) && path.extension().is_none() {
            let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
            for extension in extensions.split(';').filter(|value| value.starts_with('.')) {
                let mut name = path.as_os_str().to_os_string();
                name.push(extension);
                let candidate = PathBuf::from(name);
                if is_executable(&candidate) {
                    return std::fs::canonicalize(candidate).ok();
                }
            }
        }
        None
    }
    let path = Path::new(command);
    if path.components().count() > 1 {
        return candidate(path);
    }
    env::split_paths(&env::var_os("PATH")?)
        .find_map(|directory| candidate(&directory.join(command)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn fixture_launch_waits_for_a_temporary_writer_to_close() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        // Nix coreutils dispatches by executable name even after copying.
        let executable = directory.path().join("true");
        std::fs::copy(resolve_program("true").unwrap(), &executable).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&executable)
            .unwrap();
        let mut command = Command::new(&executable);
        command.kill_on_drop(true);
        assert_eq!(
            command.spawn().unwrap_err().raw_os_error(),
            Some(libc::ETXTBSY)
        );
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(writer);
        });
        let mut child = spawn_fixture_command(&mut command).unwrap();
        assert!(child.wait().await.unwrap().success());
        release.join().unwrap();
    }
    #[test]
    fn claude_verbose_output_uses_the_final_structured_result() {
        let value = parse_claude_response(r#"[{"type":"system"},{"type":"assistant","message":"progress"},{"type":"result","structured_output":{"subject":"Fix parsing","body":""}}]"#).unwrap();
        assert_eq!(commit::decode(&value).unwrap(), "Fix parsing");
        assert!(
            parse_claude_response(
                r#"[{"type":"result","is_error":true,"result":"Account limit"}]"#
            )
            .unwrap_err()
            .to_string()
            .contains("Account limit")
        );
    }
    #[tokio::test]
    async fn process_output_is_bounded() {
        assert!(read_capped(&b"too much data"[..], 4).await.is_err());
        assert_eq!(read_capped(&b"safe"[..], 4).await.unwrap(), b"safe");
    }
}
