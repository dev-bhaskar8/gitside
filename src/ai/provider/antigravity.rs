//! Rust port of T3 Code's dedicated Antigravity text helper (MIT).
//! ACP transport, temporary workspace, no tools, bounded output, owned cleanup.
use super::{TextRequest, agent_program};
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
        bail!("Antigravity wrote files during commit generation");
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
    profile: PathBuf,
    session: Option<String>,
    output: String,
    next_id: u64,
    signing_in: bool,
}

impl Client {
    async fn spawn(settings: &AiSettings, signing_in: bool) -> Result<Self> {
        let profile = settings
            .agent
            .antigravity_home
            .clone()
            .or_else(|| dirs::data_local_dir().map(|p| p.join("gitside/antigravity")))
            .context("could not locate Antigravity profile; set ai.agent.antigravity_home")?;
        prepare_profile(&profile).await?;
        let profile = tokio::fs::canonicalize(&profile).await?;
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
            .env("GEMINI_HOME", &profile)
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
            let harness = executable
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
        command.args(&settings.agent.args);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().context("Antigravity ACP runtime was not found; install Google's ACP Registry runtime and set ai.agent.command")?;
        let stdin = child
            .stdin
            .take()
            .context("Antigravity stdin unavailable")?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .context("Antigravity stdout unavailable")?,
        );
        let mut stderr = BufReader::new(
            child
                .stderr
                .take()
                .context("Antigravity stderr unavailable")?,
        );
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
            session: None,
            output: String::new(),
            next_id: 1,
            signing_in,
        })
    }

    async fn run(&mut self, model: Option<&str>, request: Option<&TextRequest>) -> Result<String> {
        let init = self.request("initialize", json!({"protocolVersion":1,
            "clientInfo":{"name":"gitside","version":env!("CARGO_PKG_VERSION")},
            "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}
        })).await?;
        if init["protocolVersion"] != 1 {
            bail!("Antigravity did not negotiate ACP v1");
        }
        self.request("authenticate", json!({"methodId":"oauth-personal"}))
            .await?;
        let created = self
            .request(
                "session/new",
                json!({"cwd":self.workspace_path,"mcpServers":[]}),
            )
            .await?;
        let session = created["sessionId"]
            .as_str()
            .filter(|id| valid_session_id(id))
            .context("Antigravity returned an invalid session ID")?
            .to_owned();
        self.session = Some(session.clone());
        if request.is_none() {
            return Ok(String::new());
        }
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            let options = created["configOptions"]
                .as_array()
                .and_then(|options| options.iter().find(|option| option["id"] == "model"));
            if let Some(options) = options {
                if !model_in_options(&options["options"], model) {
                    bail!("Antigravity model is unavailable for this account");
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
                    bail!("Antigravity did not advertise the selected model");
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
            bail!("Antigravity did not finish commit generation");
        }
        if self.output.trim().is_empty() {
            bail!("Antigravity returned empty commit output");
        }
        Ok(std::mem::take(&mut self.output))
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let line = tokio::select! {
                line = read_line(&mut self.stdout, &mut self.stdout_buffer) => line?.context("Antigravity stopped before completing its response")?,
                Some(line) = self.diagnostics.recv() => { self.authorization_line(&line)?; continue; }
            };
            if line.starts_with(AUTH_PREFIX) {
                self.authorization_line(&line)?;
                continue;
            }
            let event: Value =
                serde_json::from_str(&line).context("Antigravity returned invalid ACP JSON")?;
            if event.get("method").is_some() {
                self.handle_agent_message(&event).await?;
                continue;
            }
            if event["id"] != id {
                bail!("Antigravity returned an unexpected response ID");
            }
            if let Some(error) = event.get("error") {
                if error["code"] == -32000 {
                    bail!("Sign in using gitside antigravity-login before generating");
                }
                let message = error["message"].as_str().unwrap_or("ACP request failed");
                bail!(
                    "Antigravity {method} failed: {}",
                    super::shorten_subject(message, 300)
                );
            }
            return event
                .get("result")
                .cloned()
                .context("Antigravity returned no ACP result");
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
            bail!("Antigravity requested tools or user input during commit generation");
        }
        if method != "session/update" {
            return Ok(());
        }
        let params = &event["params"];
        if params["sessionId"].as_str() != self.session.as_deref() {
            return Ok(());
        }
        let update = &params["update"];
        match update["sessionUpdate"].as_str() {
            Some("tool_call" | "tool_call_update") => {
                bail!("Antigravity attempted tool work during commit generation")
            }
            Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                let text = update["content"]["text"]
                    .as_str()
                    .context("Antigravity text chunk was malformed")?;
                if self.output.len() + text.len() > MAX_OUTPUT {
                    bail!("Antigravity output exceeded the commit limit");
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
            let message =
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}});
            let _ = tokio::time::timeout(Duration::from_millis(200), self.send(message)).await;
        }
        kill_group(&self.child);
        let _ = self.child.kill().await;
        self.stderr.abort();
        self.cleanup_session();
    }
    fn cleanup_session(&self) {
        let Some(session) = &self.session else { return };
        let base = self
            .profile
            .join("antigravity-acp/conversations")
            .join(session);
        let metadata = std::fs::read(base.with_extension("meta"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if metadata.as_ref().and_then(|v| v["cwd"].as_str()) != self.workspace_path.to_str() {
            return;
        }
        for suffix in ["db", "db-wal", "db-shm", "db-journal", "meta"] {
            let _ = std::fs::remove_file(base.with_extension(suffix));
        }
        let _ = std::fs::remove_dir_all(self.profile.join("antigravity-acp/brain").join(session));
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        kill_group(&self.child);
        self.stderr.abort();
        self.cleanup_session();
    }
}
fn kill_group(child: &Child) {
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

async fn read_line(
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
            bail!("Antigravity exceeded the ACP message limit");
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
            .context("Antigravity returned non-UTF-8 output")?
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
fn model_in_options(options: &Value, model: &str) -> bool {
    options.as_array().is_some_and(|options| {
        options
            .iter()
            .any(|option| option["value"] == model || model_in_options(&option["options"], model))
    })
}
fn resolve_program(program: &str) -> Option<PathBuf> {
    if Path::new(program).components().count() > 1 {
        return std::fs::canonicalize(program).ok();
    }
    env::split_paths(&env::var_os("PATH")?)
        .find_map(|dir| std::fs::canonicalize(dir.join(program)).ok())
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
fn contains_files(path: &Path) -> Result<bool> {
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
