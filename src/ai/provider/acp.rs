//! ACP text helpers adapted from T3 Code (MIT), plus Cursor's official ACP interface.
//! ACP transport, temporary workspace, no tools, bounded output, owned cleanup.
use super::{TextRequest, agent_program, resolve_program};
use crate::config::{AgentProvider, AiSettings};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::mpsc,
    task::JoinHandle,
};

const AUTH_PREFIX: &str = "Open the following link to authenticate the ACP server: ";
pub(crate) const BROWSER_MARKER: &str = "GITSIDE_ANTIGRAVITY_AUTH_URL=";
const MAX_LINE: usize = 1_048_576;
const MAX_OUTPUT: usize = 128_000;
const REMOVED_ENV: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_LOCATION",
    "GOOGLE_CLOUD_QUOTA_PROJECT",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GCLOUD_PROJECT",
    "CLOUDSDK_CORE_PROJECT",
    "AGY_ACP_CCPA_PROJECT",
    "AGY_ACP_ENABLE_OAUTH",
    "GEMINI_HOME",
    "AGY_ACP_FORCE_FILE_STORAGE",
    "ANTIGRAVITY_HARNESS_PATH",
    "BROWSER",
];

pub(super) async fn generate(settings: &AiSettings, request: &TextRequest) -> Result<String> {
    let mut client = Client::spawn(settings, false).await?;
    let result = client
        .run(settings.agent.model.as_deref(), Some(request))
        .await;
    client.stop().await;
    if result.is_ok() && contains_files(client.workspace.path())? {
        bail!(
            "{} wrote files during commit generation",
            settings.agent.provider.label()
        );
    }
    result
}

pub(crate) async fn login(settings: &AiSettings) -> Result<()> {
    let mut settings = settings.clone();
    if settings.agent.provider != AgentProvider::Antigravity {
        settings.agent.command = None;
        settings.agent.args.clear();
    }
    settings.agent.provider = AgentProvider::Antigravity;
    let mut client = Client::spawn(&settings, true).await?;
    let result = tokio::time::timeout(Duration::from_secs(300), client.run(None, None)).await;
    client.stop().await;
    result.context("Antigravity sign-in timed out; run gitside antigravity-login again")??;
    println!("Signed in to Antigravity for Gitside.");
    Ok(())
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stdout_buffer: Vec<u8>,
    stderr: JoinHandle<()>,
    diagnostics: mpsc::Receiver<String>,
    workspace: TempDir,
    workspace_path: PathBuf,
    _runtime: TempDir,
    profile: Option<PathBuf>,
    provider: AgentProvider,
    auth_method: Option<String>,
    session: Option<String>,
    output: String,
    next_id: u64,
    signing_in: bool,
}

impl Client {
    async fn spawn(settings: &AiSettings, signing_in: bool) -> Result<Self> {
        let profile = if settings.agent.provider == AgentProvider::Antigravity {
            let profile = settings
                .agent
                .antigravity_home
                .clone()
                .or_else(|| dirs::data_local_dir().map(|p| p.join("gitside/antigravity")))
                .context("could not locate Antigravity profile; set ai.agent.antigravity_home")?;
            prepare_profile(&profile).await?;
            Some(tokio::fs::canonicalize(&profile).await?)
        } else {
            None
        };
        let auth_method = match settings.agent.provider {
            AgentProvider::Antigravity => Some("oauth-personal".to_owned()),
            AgentProvider::Cursor => Some("cursor_login".to_owned()),
            AgentProvider::Grok => Some(
                if env::var("XAI_API_KEY").is_ok_and(|key| !key.trim().is_empty()) {
                    "xai.api_key"
                } else {
                    "cached_token"
                }
                .to_owned(),
            ),
            _ => settings
                .agent
                .acp_auth_method
                .clone()
                .filter(|value| !value.trim().is_empty()),
        };
        let workspace = tempfile::tempdir()?;
        let workspace_path = std::fs::canonicalize(workspace.path())?;
        let runtime = tempfile::tempdir()?;
        let program = agent_program(settings);
        let resolved = resolve_program(&program);
        let mut command = Command::new(resolved.as_deref().unwrap_or_else(|| Path::new(&program)));
        command
            .current_dir(&workspace_path)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(profile) = &profile {
            // Isolate the account without reading/copying another application's login.
            for (name, _) in env::vars_os() {
                if REMOVED_ENV.contains(&name.to_string_lossy().to_uppercase().as_str()) {
                    command.env_remove(name);
                }
            }
            let executable = env::current_exe()?;
            let executable = executable.to_string_lossy();
            if executable.contains(if cfg!(windows) { ';' } else { ':' }) {
                bail!("Gitside executable path cannot be used for Antigravity browser handling");
            }
            let browser = format!(
                "{} __antigravity-browser %s",
                shell_words::quote(&executable)
            );
            command
                .env("GEMINI_HOME", profile)
                .env("AGY_ACP_FORCE_FILE_STORAGE", "1")
                .env("PYTHONUNBUFFERED", "1")
                .env("BROWSER", browser);
            command.env(
                if cfg!(windows) { "TEMP" } else { "TMPDIR" },
                runtime.path(),
            );
            if cfg!(windows) {
                command.env("TMP", runtime.path());
            }
            if cfg!(target_os = "linux") {
                command.arg("--uid=");
            }
            // Google's archive ships these two executables together.
            if let Some(executable) = resolve_program(&program) {
                let harness =
                    executable
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(if cfg!(windows) {
                            "localharness_external.exe"
                        } else {
                            "localharness_external"
                        });
                if harness.is_file() {
                    command.env("ANTIGRAVITY_HARNESS_PATH", harness);
                }
            }
            // Google's bundled Python may not know the host distribution's CA path.
            if cfg!(target_os = "linux")
                && env::var_os("SSL_CERT_FILE").is_none()
                && env::var_os("SSL_CERT_DIR").is_none()
            {
                let mut candidates = vec![env::var_os("NIX_SSL_CERT_FILE").map(PathBuf::from)];
                candidates.extend(
                    [
                        "/etc/ssl/certs/ca-bundle.crt",
                        "/etc/ssl/certs/ca-certificates.crt",
                        "/etc/pki/tls/certs/ca-bundle.crt",
                        "/etc/ssl/cert.pem",
                    ]
                    .into_iter()
                    .map(|path| Some(PathBuf::from(path))),
                );
                if let Some(certificate) = candidates
                    .into_iter()
                    .flatten()
                    .find(|path| path.is_absolute() && path.is_file())
                {
                    command.env("SSL_CERT_FILE", certificate);
                }
            }
        }
        command.args(&settings.agent.args);
        match settings.agent.provider {
            AgentProvider::Cursor => {
                command.args(["--mode", "ask", "acp"]);
            }
            AgentProvider::Grok => {
                command.args(["--permission-mode", "default", "agent", "stdio"]);
            }
            _ => {}
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().with_context(|| format!("{} ACP runtime was not found; install it and set ai.agent.command if it is not on PATH", settings.agent.provider.label()))?;
        let stdin = child.stdin.take().context("ACP stdin unavailable")?;
        let stdout = BufReader::new(child.stdout.take().context("ACP stdout unavailable")?);
        let mut stderr = BufReader::new(child.stderr.take().context("ACP stderr unavailable")?);
        let (tx, diagnostics) = mpsc::channel(16);
        let stderr = tokio::spawn(async move {
            let mut buffer = Vec::new();
            while let Ok(Some(line)) = read_line(&mut stderr, &mut buffer).await {
                // Drain diagnostics continuously; never log provider credentials.
                if (line.starts_with(BROWSER_MARKER) || line.starts_with(AUTH_PREFIX))
                    && tx.send(line).await.is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            stdout,
            stdout_buffer: Vec::new(),
            stderr,
            diagnostics,
            workspace,
            workspace_path,
            _runtime: runtime,
            profile,
            provider: settings.agent.provider,
            auth_method,
            session: None,
            output: String::new(),
            next_id: 1,
            signing_in,
        })
    }

    fn label(&self) -> &'static str {
        self.provider.label()
    }

    async fn run(&mut self, model: Option<&str>, request: Option<&TextRequest>) -> Result<String> {
        let mut params = json!({"protocolVersion":1,
            "clientInfo":{"name":"gitside","version":env!("CARGO_PKG_VERSION")},
            "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}
        });
        if self.provider == AgentProvider::Grok {
            params["_meta"] = json!({"clientType":"extension"});
        }
        let init = self.request("initialize", params).await?;
        if init["protocolVersion"] != 1 {
            bail!("{} did not negotiate ACP v1", self.label());
        }
        if let Some(method) = self.auth_method.clone() {
            self.request("authenticate", json!({"methodId":method}))
                .await?;
        }
        let created = self
            .request(
                "session/new",
                json!({"cwd":self.workspace_path,"mcpServers":[]}),
            )
            .await?;
        let session = created["sessionId"]
            .as_str()
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 512
                    && !id.chars().any(char::is_control)
                    && (self.provider != AgentProvider::Antigravity || valid_session_id(id))
            })
            .with_context(|| format!("{} returned an invalid session ID", self.label()))?
            .to_owned();
        self.session = Some(session.clone());
        if request.is_none() {
            return Ok(String::new());
        }
        if self.provider == AgentProvider::Cursor {
            self.request(
                "session/set_mode",
                json!({"sessionId":session,"modeId":"ask"}),
            )
            .await?;
        }
        if let Some(model) = model.map(str::trim).filter(|value| {
            !value.is_empty()
                && *value != "default"
                && !(self.provider == AgentProvider::Grok && *value == "grok-build")
        }) {
            let options = created["configOptions"]
                .as_array()
                .and_then(|options| options.iter().find(|option| option["id"] == "model"));
            if let Some(options) = options {
                if !model_in_options(&options["options"], model) {
                    bail!("{} model is unavailable for this account", self.label());
                }
                self.request(
                    "session/set_config_option",
                    json!({"sessionId":session,"configId":"model","value":model}),
                )
                .await?;
            } else {
                let models = &created["models"]["availableModels"];
                if !models
                    .as_array()
                    .is_some_and(|models| models.iter().any(|entry| entry["modelId"] == model))
                {
                    bail!("{} did not advertise the selected model", self.label());
                }
                self.request(
                    "session/set_model",
                    json!({"sessionId":session,"modelId":model}),
                )
                .await?;
            }
        }
        let result = self
            .request(
                "session/prompt",
                json!({"sessionId":session,
            "prompt":[{"type":"text","text":request.unwrap().combined_prompt()}]}),
            )
            .await?;
        if result["stopReason"] != "end_turn" {
            bail!("{} did not finish commit generation", self.label());
        }
        if self.output.trim().is_empty() {
            bail!("{} returned empty commit output", self.label());
        }
        Ok(std::mem::take(&mut self.output))
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let mut params = params;
        let prompt_id = (self.provider == AgentProvider::Grok && method == "session/prompt")
            .then(|| uuid::Uuid::now_v7().to_string());
        if let Some(prompt_id) = &prompt_id {
            params["_meta"] = json!({"promptId":prompt_id,"requestId":prompt_id});
        }
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let line = tokio::select! {
                line = read_line(&mut self.stdout, &mut self.stdout_buffer) => line?.with_context(|| format!("{} stopped before completing its response; check its login and CLI version", self.label()))?,
                Some(line) = self.diagnostics.recv() => { self.authorization_line(&line)?; continue; }
            };
            if line.starts_with(AUTH_PREFIX) {
                self.authorization_line(&line)?;
                continue;
            }
            let event: Value = serde_json::from_str(&line)
                .with_context(|| format!("{} returned invalid ACP JSON", self.label()))?;
            if event["jsonrpc"] != "2.0" {
                bail!("{} returned an invalid RPC envelope", self.label());
            }
            if event.get("method").is_some() {
                self.handle_agent_message(&event).await?;
                if let Some(prompt_id) = &prompt_id {
                    if let Some(result) =
                        grok_completion(&event, self.session.as_deref(), prompt_id)?
                    {
                        return Ok(result);
                    }
                }
                continue;
            }
            if event["id"] != id {
                bail!("{} returned an unexpected response ID", self.label());
            }
            if let Some(error) = event.get("error") {
                if self.provider == AgentProvider::Antigravity && error["code"] == -32000 {
                    bail!("Sign in using gitside antigravity-login before generating");
                }
                let message = error["message"].as_str().unwrap_or("ACP request failed");
                bail!(
                    "{} {method} failed: {}",
                    self.label(),
                    super::shorten_subject(message, 300)
                );
            }
            return event
                .get("result")
                .cloned()
                .with_context(|| format!("{} returned no ACP result", self.label()));
        }
    }

    async fn handle_agent_message(&mut self, event: &Value) -> Result<()> {
        let method = event["method"].as_str().unwrap_or_default();
        if let Some(id) = event.get("id") {
            let response = if method == "session/request_permission" {
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}})
            } else {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Tools and user input are disabled for commit generation"}})
            };
            self.send(response).await?;
            bail!(
                "{} requested tools or user input during commit generation",
                self.label()
            );
        }
        if method != "session/update" {
            return Ok(());
        }
        let params = &event["params"];
        if params["sessionId"].as_str() != self.session.as_deref() {
            return Ok(());
        }
        if self.provider == AgentProvider::Grok
            && params["_meta"]["promptId"]
                .as_str()
                .is_some_and(|id| id.starts_with("task-completed-"))
        {
            return Ok(());
        }
        let update = &params["update"];
        match update["sessionUpdate"].as_str() {
            Some("tool_call" | "tool_call_update") => {
                bail!(
                    "{} attempted tool work during commit generation",
                    self.label()
                )
            }
            Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                let text = update["content"]["text"]
                    .as_str()
                    .context("ACP text chunk was malformed")?;
                if self.output.len() + text.len() > MAX_OUTPUT {
                    bail!("{} output exceeded the commit limit", self.label());
                }
                self.output.push_str(text);
            }
            _ => {}
        }
        Ok(())
    }

    fn authorization_line(&self, line: &str) -> Result<()> {
        if !self.signing_in {
            bail!("Antigravity needs sign-in; run gitside antigravity-login");
        }
        let url = if let Some(raw) = line.strip_prefix(BROWSER_MARKER) {
            serde_json::from_str::<String>(raw).context("invalid Antigravity sign-in URL")?
        } else {
            line.trim_start_matches(AUTH_PREFIX).to_owned()
        };
        let parsed = reqwest::Url::parse(&url).context("invalid Antigravity sign-in URL")?;
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("accounts.google.com")
            || parsed.path() != "/o/oauth2/v2/auth"
        {
            bail!("Antigravity returned an unexpected sign-in URL");
        }
        println!(
            "Open this Google sign-in link in your browser:\n{url}\nWaiting for Google sign-in…"
        );
        Ok(())
    }

    async fn send(&mut self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn stop(&mut self) {
        if let Some(session) = &self.session {
            let mut message =
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}});
            if self.provider == AgentProvider::Grok {
                message["params"]["_meta"] = json!({"cancelTrigger":"ctrl_c"});
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), self.send(message)).await;
        }
        kill_group(&self.child);
        let _ = self.child.kill().await;
        self.stderr.abort();
        self.cleanup_session();
    }
    fn cleanup_session(&self) {
        let Some(session) = &self.session else { return };
        let Some(profile) = &self.profile else { return };
        let base = profile.join("antigravity-acp/conversations").join(session);
        let metadata = std::fs::read(base.with_extension("meta"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if metadata.as_ref().and_then(|v| v["cwd"].as_str()) != self.workspace_path.to_str() {
            return;
        }
        for suffix in ["db", "db-wal", "db-shm", "db-journal", "meta"] {
            let _ = std::fs::remove_file(base.with_extension(suffix));
        }
        let _ = std::fs::remove_dir_all(profile.join("antigravity-acp/brain").join(session));
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        kill_group(&self.child);
        self.stderr.abort();
        self.cleanup_session();
    }
}
pub(super) fn kill_group(child: &Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // This child started its own process group; no unrelated processes join it.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = child;
}

pub(super) async fn read_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    bytes: &mut Vec<u8>,
) -> Result<Option<String>> {
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        let length = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if bytes.len() + length > MAX_LINE {
            bail!("AI provider exceeded the ACP message limit");
        }
        let done = chunk[length - 1] == b'\n';
        bytes.extend_from_slice(&chunk[..length]);
        reader.consume(length);
        if done {
            break;
        }
    }
    Ok(Some(
        String::from_utf8(std::mem::take(bytes))
            .context("ACP provider returned non-UTF-8 output")?
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
    ))
}

fn valid_session_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

// Grok sometimes completes through an extension notification before its prompt RPC.
// Correlate both session and prompt: background wakes must never finish this request.
fn grok_completion(event: &Value, session: Option<&str>, prompt: &str) -> Result<Option<Value>> {
    let method = event["method"].as_str().unwrap_or_default();
    let params = &event["params"];
    if params["sessionId"].as_str() != session || event.get("id").is_some() {
        return Ok(None);
    }
    let (id, reason) = match method {
        "x.ai/session/prompt_complete" | "_x.ai/session/prompt_complete" => {
            (&params["promptId"], &params["stopReason"])
        }
        "x.ai/session_notification" | "_x.ai/session_notification" | "_x.ai/session/update"
            if params["update"]["sessionUpdate"] == "turn_completed" =>
        {
            let update = &params["update"];
            (
                update.get("prompt_id").unwrap_or(&update["promptId"]),
                update.get("stop_reason").unwrap_or(&update["stopReason"]),
            )
        }
        _ => return Ok(None),
    };
    if id.as_str() != Some(prompt) {
        return Ok(None);
    }
    let reason = reason.as_str().unwrap_or("end_turn");
    if reason == "rate_limit" {
        bail!("Grok usage limit reached; try again later");
    }
    Ok(Some(json!({"stopReason":reason})))
}
fn model_in_options(options: &Value, model: &str) -> bool {
    options.as_array().is_some_and(|options| {
        options
            .iter()
            .any(|option| option["value"] == model || model_in_options(&option["options"], model))
    })
}
async fn prepare_profile(profile: &Path) -> Result<()> {
    let config = profile.join("config");
    for name in ["hooks.json", "mcp_config.json"] {
        let path = config.join(name);
        if path.exists() {
            if tokio::fs::metadata(&path).await?.len() > 64_000 {
                bail!("Antigravity profile has unsupported hook/MCP configuration");
            }
            let data = tokio::fs::read(&path).await?;
            let value: Value = serde_json::from_slice(&data)
                .context("invalid Antigravity hook/MCP configuration")?;
            let entry = value
                .get(if name == "hooks.json" {
                    "hooks"
                } else {
                    "mcpServers"
                })
                .unwrap_or(&value);
            if !entry.as_object().is_some_and(|value| value.is_empty()) {
                bail!(
                    "Antigravity text generation requires a profile without global hooks or MCP servers"
                );
            }
        }
    }
    let directory = profile.join("antigravity-acp");
    tokio::fs::create_dir_all(&directory).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [profile, directory.as_path()] {
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
        }
    }
    tokio::fs::write(
        directory.join("settings.json"),
        "{\"auth\":{\"type\":\"oauth-personal\"}}\n",
    )
    .await?;
    Ok(())
}
pub(super) fn contains_files(path: &Path) -> Result<bool> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if contains_files(&entry.path())? {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AiMode;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn line_buffer_survives_cancelled_reads_and_enforces_limits() {
        let (mut writer, reader) = tokio::io::duplex(MAX_LINE + 16);
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        writer.write_all(b"{\"partial\":").await.unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                read_line(&mut reader, &mut buffer)
            )
            .await
            .is_err()
        );
        writer.write_all(b"true}\n").await.unwrap();
        assert_eq!(
            read_line(&mut reader, &mut buffer).await.unwrap().unwrap(),
            "{\"partial\":true}"
        );
        writer.write_all(&vec![b'x'; MAX_LINE + 1]).await.unwrap();
        assert!(read_line(&mut reader, &mut buffer).await.is_err());
    }

    #[tokio::test]
    async fn profile_rejects_hooks_without_overwriting_them() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        tokio::fs::create_dir(&config).await.unwrap();
        let hooks = config.join("hooks.json");
        tokio::fs::write(&hooks, r#"{"hooks":{"BeforeAgent":["do-something"]}}"#)
            .await
            .unwrap();
        assert!(prepare_profile(directory.path()).await.is_err());
        assert!(
            tokio::fs::read_to_string(hooks)
                .await
                .unwrap()
                .contains("do-something")
        );
    }

    #[cfg(unix)]
    async fn fixture(mode: &str) -> (TempDir, AiSettings) {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("fake-acp");
        let source = r##"#!/usr/bin/env python3
import json,os,sys,time,pathlib
mode = MODE
profile = pathlib.Path(os.environ['GEMINI_HOME'])
assert os.environ['AGY_ACP_FORCE_FILE_STORAGE'] == '1'
assert 'GEMINI_API_KEY' not in os.environ
profile.joinpath('test-workspace').write_text(os.getcwd())
profile.joinpath('test-pid').write_text(str(os.getpid()))
session = '22222222-2222-4222-8222-222222222222'
def send(value):
 print(json.dumps(value),flush=True)
for line in sys.stdin:
 request=json.loads(line)
 method=request.get('method')
 if method=='session/cancel': break
 result={}
 if method=='initialize':
  assert request['params']['clientInfo']['name']=='gitside'
  assert request['params']['clientCapabilities']['terminal'] is False
  result={'protocolVersion':1}
 elif method=='authenticate':
  assert request['params']['methodId']=='oauth-personal'
  if mode=='hang':
   time.sleep(30)
  if mode=='auth':
   print('Open the following link to authenticate the ACP server: https://accounts.google.com/o/oauth2/v2/auth?client_id=test',flush=True)
   time.sleep(30)
 elif method=='session/new':
  assert request['params']['mcpServers']==[]
  base=profile/'antigravity-acp'/'conversations'
  base.mkdir(parents=True,exist_ok=True)
  base.joinpath(session+'.meta').write_text(json.dumps({'cwd':os.getcwd()}))
  base.joinpath(session+'.db').write_text('temporary transcript')
  result={'sessionId': '../bad' if mode=='bad-session' else session,
    'configOptions':[{'id':'model','type':'select','currentValue':'default','options':[{'value':'test-model','name':'Test'}]}]}
 elif method=='session/set_config_option':
  assert request['params']['configId']=='model'
  assert request['params']['value']=='test-model'
 elif method=='session/prompt':
  assert 'Return a JSON object' in request['params']['prompt'][0]['text']
  if mode=='permission':
   send({'jsonrpc':'2.0','id':'tool-1','method':'session/request_permission','params':{'sessionId':session,'options':[]}})
   reply=json.loads(sys.stdin.readline())
   assert reply['result']['outcome']['outcome']=='cancelled'
   profile.joinpath('test-denied').write_text('yes')
   continue
  if mode=='files': pathlib.Path('unexpected.txt').write_text('unexpected')
  chunks=['{"subject":"Add Antigravity support.",','"body":"Use structured output."}']
  if mode=='overflow': chunks=['x'*128001]
  for text in chunks:
   send({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':session,'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}}})
  result={'stopReason':'end_turn'}
 send({'jsonrpc':'2.0','id':request['id'],'result':result})
"##.replace("MODE", &serde_json::to_string(mode).unwrap());
        tokio::fs::write(&script, source).await.unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
        let mut settings = AiSettings {
            enabled: true,
            mode: AiMode::Agent,
            timeout_secs: 2,
            ..AiSettings::default()
        };
        settings.agent.provider = AgentProvider::Antigravity;
        settings.agent.command = Some(script.to_string_lossy().into_owned());
        settings.agent.antigravity_home = Some(directory.path().join("profile"));
        settings.agent.model = Some("test-model".into());
        (directory, settings)
    }
    fn task() -> TextRequest {
        TextRequest {
            system: crate::ai::commit::SYSTEM.into(),
            prompt: "Changed files: feature.rs".into(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn acp_generation_uses_account_models_and_cleans_owned_sessions() {
        let (_directory, settings) = fixture("success").await;
        let output = generate(&settings, &task()).await.unwrap();
        assert_eq!(
            crate::ai::commit::decode(&output).unwrap(),
            "Add Antigravity support\n\nUse structured output."
        );
        let profile = settings.agent.antigravity_home.unwrap();
        let workspace = tokio::fs::read_to_string(profile.join("test-workspace"))
            .await
            .unwrap();
        assert!(!Path::new(&workspace).exists());
        let conversations = profile.join("antigravity-acp/conversations");
        assert_eq!(std::fs::read_dir(conversations).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn acp_helper_rejects_tool_work_writes_auth_and_bad_output() {
        for (mode, expected) in [
            ("permission", "tools or user input"),
            ("files", "wrote files"),
            ("auth", "sign-in"),
            ("bad-session", "invalid session"),
            ("overflow", "output exceeded"),
        ] {
            let (_directory, settings) = fixture(mode).await;
            let error = generate(&settings, &task()).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{mode}: {error:#}");
        }
    }

    #[cfg(unix)]
    async fn native_fixture(provider: AgentProvider, mode: &str) -> (TempDir, AiSettings) {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("native-acp");
        let source = r##"#!/usr/bin/env python3
import os,sys,json,pathlib
provider=PROVIDER
mode=MODE
assert not list(pathlib.Path.cwd().iterdir())
expected={'cursor':['--mode','ask','acp'],'grok':['--permission-mode','default','agent','stdio'],'acp':[]}
assert sys.argv[1:]==expected[provider]
session='native/session-1'
def send(value): print(json.dumps(value),flush=True)
for line in sys.stdin:
 q=json.loads(line);m=q.get('method');p=q.get('params',{});result={}
 if m=='session/cancel': break
 if m=='initialize':
  assert p['clientCapabilities']=={'fs':{'readTextFile':False,'writeTextFile':False},'terminal':False}
  if provider=='grok':assert p['_meta']=={'clientType':'extension'}
  result={'protocolVersion':1}
 if m=='authenticate':
  method={'cursor':'cursor_login','grok':'xai.api_key' if os.environ.get('XAI_API_KEY','').strip() else 'cached_token','acp':'existing_login'}[provider]
  assert p['methodId']==method
  if mode=='auth':send({'jsonrpc':'2.0','id':q['id'],'error':{'code':-32000,'message':'Please sign in first'}});continue
 if m=='session/new':
  assert p['mcpServers']==[] and pathlib.Path(p['cwd']).resolve()==pathlib.Path.cwd()
  result={'sessionId':session,'models':{'availableModels':[{'modelId':'model-1'}]}}
 if m=='session/set_mode':assert p['modeId']=='ask'
 if m=='session/set_model':assert p['modelId']=='model-1'
 if m=='session/prompt':
  assert 'Return a JSON object' in p['prompt'][0]['text']
  if mode=='tools':
   send({'jsonrpc':'2.0','id':'tool','method':'session/request_permission','params':{'sessionId':session,'options':[]}})
   reply=json.loads(sys.stdin.readline());assert reply['result']['outcome']['outcome']=='cancelled';continue
  if mode=='input':
   send({'jsonrpc':'2.0','id':'input','method':'cursor/ask_question','params':{'sessionId':session}})
   reply=json.loads(sys.stdin.readline());assert reply['error']['code']==-32601;continue
  for sid,text,kind in [('unrelated','ignore','agent_message_chunk'),(session,'hidden','agent_thought_chunk'),(session,'{"subject":"Add ACP providers.","body":"Reuse native authentication."}','agent_message_chunk')]:
   send({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':sid,'update':{'sessionUpdate':kind,'content':{'type':'text','text':text}}}})
  if provider=='grok' and mode in ['notification','rate-limit']:
   prompt=p['_meta']['promptId']
   send({'jsonrpc':'2.0','method':'_x.ai/session/prompt_complete','params':{'sessionId':session,'promptId':'task-completed-ignore','stopReason':'cancelled'}})
   send({'jsonrpc':'2.0','method':'_x.ai/session_notification','params':{'sessionId':session,'update':{'sessionUpdate':'turn_completed','prompt_id':prompt,'stop_reason':'rate_limit' if mode=='rate-limit' else 'end_turn'}}})
   continue
  result={'stopReason':'cancelled' if mode=='cancelled' else 'end_turn'}
 send({'jsonrpc':'2.0','id':q['id'],'result':result})
"##.replace("PROVIDER", &json!(match provider { AgentProvider::Cursor=>"cursor",AgentProvider::Grok=>"grok",_=>"acp" }).to_string()).replace("MODE",&json!(mode).to_string());
        tokio::fs::write(&script, source).await.unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
        let mut settings = AiSettings::default();
        settings.agent.provider = provider;
        settings.agent.command = Some(script.to_string_lossy().into_owned());
        settings.agent.model = Some("model-1".into());
        settings.agent.acp_auth_method = Some("existing_login".into());
        (directory, settings)
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cursor_grok_and_custom_acp_use_native_auth_models_and_text_events() {
        for provider in [
            AgentProvider::Cursor,
            AgentProvider::Grok,
            AgentProvider::Acp,
        ] {
            let (_directory, settings) = native_fixture(provider, "success").await;
            let output = generate(&settings, &task()).await.unwrap();
            assert_eq!(
                crate::ai::commit::decode(&output).unwrap(),
                "Add ACP providers\n\nReuse native authentication."
            );
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn grok_extension_completion_is_correlated_and_propagates_usage_limits() {
        let (_directory, settings) = native_fixture(AgentProvider::Grok, "notification").await;
        assert!(
            generate(&settings, &task())
                .await
                .unwrap()
                .contains("Add ACP providers")
        );
        let (_directory, settings) = native_fixture(AgentProvider::Grok, "rate-limit").await;
        assert!(
            generate(&settings, &task())
                .await
                .unwrap_err()
                .to_string()
                .contains("usage limit")
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn native_acp_errors_permissions_and_invalid_models_are_not_drafts() {
        for provider in [
            AgentProvider::Cursor,
            AgentProvider::Grok,
            AgentProvider::Acp,
        ] {
            for (mode, expected) in [
                ("auth", "sign in"),
                ("tools", "tools or user input"),
                ("input", "tools or user input"),
                ("cancelled", "did not finish"),
            ] {
                let (_directory, settings) = native_fixture(provider, mode).await;
                let error =
                    tokio::time::timeout(Duration::from_secs(5), generate(&settings, &task()))
                        .await
                        .unwrap()
                        .unwrap_err();
                assert!(
                    error.to_string().contains(expected),
                    "{provider:?} {mode}: {error:#}"
                );
            }
            let (_directory, mut settings) = native_fixture(provider, "success").await;
            settings.agent.model = Some("unavailable".into());
            assert!(
                generate(&settings, &task())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("did not advertise")
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_a_timed_out_acp_request_terminates_its_process() {
        let (_directory, settings) = fixture("hang").await;
        let mut client = Client::spawn(&settings, false).await.unwrap();
        client
            .request(
                "initialize",
                json!({"clientInfo":{"name":"gitside"},"clientCapabilities":{"terminal":false}}),
            )
            .await
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            client.request("authenticate", json!({"methodId":"oauth-personal"})),
        )
        .await;
        assert!(result.is_err());
        drop(client);
        let profile = settings.agent.antigravity_home.unwrap();
        let pid: i32 = tokio::fs::read_to_string(profile.join("test-pid"))
            .await
            .unwrap()
            .parse()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
