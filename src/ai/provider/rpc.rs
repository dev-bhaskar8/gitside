//! T3 Code's Pi RPC and Muse MSP text-generation flows, ported to Rust (MIT).
//! Each request owns an empty workspace and an ephemeral process, using CLI auth.
use super::{TextRequest, acp, agent_program, resolve_program};
use crate::config::{AgentProvider, AiSettings};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::process::Stdio;
use tempfile::TempDir;
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub(super) async fn generate(settings: &AiSettings, request: &TextRequest) -> Result<String> {
    let mut process = Process::spawn(settings)?;
    let result = match settings.agent.provider {
        AgentProvider::Pi => process.pi(settings, request).await,
        AgentProvider::Muse => process.muse(settings, request).await,
        _ => unreachable!(),
    };
    acp::kill_group(&process.child);
    let _ = process.child.kill().await;
    if result.is_ok() && acp::contains_files(process.workspace.path())? {
        bail!(
            "{} wrote files during commit generation",
            process.provider.label()
        );
    }
    result
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    buffer: Vec<u8>,
    workspace: TempDir,
    provider: AgentProvider,
    next_id: u64,
    settled: bool,
    session: Option<String>,
    turn: Option<String>,
    items: Vec<Value>,
    completed: Option<String>,
}

impl Process {
    fn spawn(settings: &AiSettings) -> Result<Self> {
        let workspace = tempfile::tempdir()?;
        let program = agent_program(settings);
        let resolved = resolve_program(&program);
        let mut command = Command::new(
            resolved
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new(&program)),
        );
        command
            .current_dir(std::fs::canonicalize(workspace.path())?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        match settings.agent.provider {
            AgentProvider::Pi => {
                // Explicit extensions/tools can bypass --no-extensions/--no-tools.
                // Keep unattended utility runs free of caller-supplied launch behavior.
                if !settings.agent.args.is_empty() {
                    bail!(
                        "Pi commit generation does not accept launch arguments; set ai.agent.model to provider/model instead"
                    );
                }
                command.args([
                    "--mode",
                    "rpc",
                    "--no-session",
                    "--no-extensions",
                    "--no-tools",
                ]);
            }
            AgentProvider::Muse => {
                if !settings.agent.args.is_empty() {
                    bail!(
                        "Muse commit generation does not accept launch arguments; set ai.agent.model instead"
                    );
                }
                command.args(["serve", "--disable-shell", "--disable-write"]);
                command.env("MUSE_NO_AUTO_UPDATE", "1");
                // Like T3, use Muse's saved login rather than an ambient Meta key.
                for (name, _) in std::env::vars_os() {
                    if name.to_string_lossy().eq_ignore_ascii_case("META_API_KEY") {
                        command.env_remove(name);
                    }
                }
            }
            _ => unreachable!(),
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().with_context(|| {
            format!(
                "Could not start {}; install it and sign in first",
                settings.agent.provider.label()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .context("AI provider stdin unavailable")?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .context("AI provider stdout unavailable")?,
        );
        Ok(Self {
            child,
            stdin,
            stdout,
            buffer: Vec::new(),
            workspace,
            provider: settings.agent.provider,
            next_id: 1,
            settled: false,
            session: None,
            turn: None,
            items: Vec::new(),
            completed: None,
        })
    }

    async fn pi(&mut self, settings: &AiSettings, request: &TextRequest) -> Result<String> {
        if let Some(model) = settings
            .agent
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "default")
        {
            let (provider, model_id) = model
                .split_once('/')
                .filter(|(p, m)| !p.is_empty() && !m.is_empty())
                .context("Pi models must use provider/model format")?;
            self.request("set_model", json!({"provider":provider,"modelId":model_id})).await
                .context("Pi rejected the selected model; extension models are unavailable because text generation disables extensions")?;
        }
        self.request("prompt", json!({"message":request.combined_prompt()}))
            .await?;
        while !self.settled {
            let event = self.receive().await?;
            self.observe(&event).await?;
        }
        let reply = self.request("get_last_assistant_text", json!({})).await?;
        reply["text"]
            .as_str()
            .filter(|text| !text.trim().is_empty() && text.len() <= 128_000)
            .map(str::to_owned)
            .context("Pi returned empty or oversized commit output")
    }

    async fn muse(&mut self, settings: &AiSettings, request: &TextRequest) -> Result<String> {
        let init = self.request("initialize", json!({
            "clientInfo":{"name":"gitside","title":"Gitside","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"requestedCapabilities":[]}
        })).await?;
        if init["schema"]["version"] != 1 || !init["schema"]["fingerprint"].is_string() {
            bail!("Muse returned an unsupported MSP schema; update Muse Code");
        }
        self.send(json!({"jsonrpc":"2.0","method":"initialized"}))
            .await?;
        let mut params = json!({"commandId":uuid::Uuid::now_v7().to_string(),
            "workspaceRoot":std::fs::canonicalize(self.workspace.path())?,
            "providerId":"meta","approvalMode":"denyUnmatched"});
        if let Some(model) = settings
            .agent
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty() && *model != "default")
        {
            params["modelId"] = json!(model);
        }
        let started = self.request("session/start", params).await?;
        self.session = Some(
            started["session"]["sessionId"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 512)
                .context("Muse returned no session ID")?
                .to_owned(),
        );
        let turn = uuid::Uuid::now_v7().to_string();
        self.turn = Some(turn.clone());
        self.request(
            "turn/start",
            json!({"commandId":turn,"sessionId":self.session,
            "input":[{"type":"text","text":request.combined_prompt()}]}),
        )
        .await?;
        while self.completed.is_none() {
            let event = self.receive().await?;
            self.observe(&event).await?;
        }
        self.completed
            .take()
            .filter(|text| !text.trim().is_empty())
            .context("Muse returned empty commit output")
    }

    async fn send(&mut self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<Value> {
        let line = acp::read_line(&mut self.stdout, &mut self.buffer)
            .await?
            .with_context(|| {
                format!(
                    "{} exited before generation completed; check its login and version",
                    self.provider.label()
                )
            })?;
        serde_json::from_str(&line)
            .with_context(|| format!("{} returned invalid protocol JSON", self.provider.label()))
    }

    async fn request(&mut self, method: &str, mut params: Value) -> Result<Value> {
        let id = format!("gitside-{}", self.next_id);
        self.next_id += 1;
        let frame = if self.provider == AgentProvider::Pi {
            params["id"] = json!(id);
            params["type"] = json!(method);
            params
        } else {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        };
        let command_id = frame["params"].get("commandId").cloned();
        self.send(frame).await?;
        loop {
            let event = self.receive().await?;
            let response = if self.provider == AgentProvider::Pi {
                event["type"] == "response"
            } else {
                event.get("method").is_none() && event.get("id").is_some()
            };
            if response {
                if event["id"] != id {
                    bail!(
                        "{} returned an unexpected response ID",
                        self.provider.label()
                    );
                }
                if self.provider == AgentProvider::Pi {
                    if event["command"] != method {
                        bail!("Pi returned an unexpected response command");
                    }
                    if event["success"] != true {
                        bail!(
                            "Pi {method} failed: {}",
                            super::shorten_subject(
                                event["error"].as_str().unwrap_or("request failed"),
                                300
                            )
                        );
                    }
                    return Ok(event["data"].clone());
                }
                if event["jsonrpc"] != "2.0" {
                    bail!("Muse returned an invalid RPC envelope");
                }
                if let Some(error) = event.get("error") {
                    bail!(
                        "Muse {method} failed: {}",
                        super::shorten_subject(
                            error["message"]
                                .as_str()
                                .unwrap_or("request failed; check muse login"),
                            300
                        )
                    );
                }
                if let Some(echo) = event["result"].get("commandId") {
                    if command_id.as_ref() != Some(echo) {
                        bail!("Muse acknowledged an unexpected command ID");
                    }
                }
                return event
                    .get("result")
                    .cloned()
                    .context("Muse returned no RPC result");
            }
            self.observe(&event).await?;
            // MSP may deliver terminal notifications before the turn acknowledgement.
            if method == "turn/start" && self.completed.is_some() {
                return Ok(Value::Null);
            }
        }
    }

    async fn observe(&mut self, event: &Value) -> Result<()> {
        if self.provider == AgentProvider::Pi {
            match event["type"].as_str() {
                Some("agent_settled") => self.settled = true,
                Some("tool_execution_start" | "extension_ui_request") => {
                    bail!("Pi requested tools or user input during commit generation")
                }
                Some("message_end")
                    if event["message"]["role"] == "assistant"
                        && event["message"]["stopReason"] == "error" =>
                {
                    bail!(
                        "Pi generation failed: {}",
                        super::shorten_subject(
                            event["message"]["errorMessage"]
                                .as_str()
                                .unwrap_or("provider error"),
                            300
                        )
                    );
                }
                _ => {}
            }
            return Ok(());
        }
        if event["jsonrpc"] != "2.0" {
            bail!("Muse returned an invalid RPC envelope");
        }
        let method = event["method"]
            .as_str()
            .context("Muse returned an unexpected response")?;
        if let Some(id) = event.get("id") {
            self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Interactive requests are disabled for commit generation"}})).await?;
            bail!("Muse requested an interactive action during text generation");
        }
        let params = &event["params"];
        if method == "view/gap" {
            bail!("Muse lost part of its response");
        }
        if params["sessionId"].as_str() != self.session.as_deref() {
            return Ok(());
        }
        match method {
            "approval/requested" | "userInput/requested" => {
                let (method, params) = if method == "approval/requested" {
                    let deny = params["availableChoices"].as_array().and_then(|choices| {
                        choices.iter().find(|choice| choice["decision"] == "denied")
                    });
                    let Some(deny) = deny else {
                        bail!("Muse requested an interactive action without a deny option");
                    };
                    (
                        "approval/decide",
                        json!({"sessionId":self.session,"approvalId":params["approvalId"],"requirementId":params["currentRequirementId"],"choiceId":deny["choiceId"]}),
                    )
                } else {
                    (
                        "userInput/cancel",
                        json!({"sessionId":self.session,"userInputId":params["userInputId"],"reason":"Commit generation cannot request user input"}),
                    )
                };
                let mut params = params;
                params["commandId"] = json!(uuid::Uuid::now_v7().to_string());
                let id = format!("gitside-{}", self.next_id);
                self.next_id += 1;
                self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                    .await?;
                bail!("Muse requested an interactive action during text generation");
            }
            "item/completed" | "item/updated" => {
                let item = &params["item"];
                if item["turnId"].as_str() != self.turn.as_deref() || item["kind"] != "agentMessage"
                {
                    return Ok(());
                }
                let id = item["itemId"].as_str().context("Muse item had no ID")?;
                let revision = item["revision"]
                    .as_u64()
                    .context("Muse item had no revision")?;
                if let Some(previous) = self
                    .items
                    .iter_mut()
                    .find(|previous| previous["itemId"] == id)
                {
                    if previous["revision"]
                        .as_u64()
                        .is_none_or(|old| revision > old)
                    {
                        *previous = item.clone();
                    }
                } else {
                    self.items.push(item.clone());
                }
                if self.items.len() > 128
                    || self
                        .items
                        .iter()
                        .map(|item| item.to_string().len())
                        .sum::<usize>()
                        > 262_144
                {
                    bail!("Muse exceeded the text generation output limit");
                }
            }
            "turn/completed" if params["turnId"].as_str() == self.turn.as_deref() => {
                if params["terminal"] != "completed" {
                    bail!(
                        "Muse generation failed: {}",
                        super::shorten_subject(
                            params["error"]["message"]
                                .as_str()
                                .unwrap_or("turn did not complete"),
                            300
                        )
                    );
                }
                let answer = self
                    .items
                    .iter()
                    .rev()
                    .find(|item| item["retracted"] != true)
                    .context("Muse returned no assistant message")?;
                if answer["truncated"] == true {
                    bail!("Muse returned a truncated response");
                }
                let text = answer["text"]
                    .as_str()
                    .filter(|text| text.len() <= 128_000)
                    .context("Muse returned invalid or oversized text")?;
                self.completed = Some(text.to_owned());
            }
            _ => {}
        }
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        acp::kill_group(&self.child);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    async fn fixture(provider: AgentProvider, mode: &str) -> (TempDir, AiSettings) {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("fake-provider");
        let source = r##"#!/usr/bin/env python3
import sys,json,os,pathlib,time
provider=PROVIDER
mode=MODE
assert not list(pathlib.Path.cwd().iterdir())
if provider=='pi':
 assert sys.argv[1:]==['--mode','rpc','--no-session','--no-extensions','--no-tools']
else:
 assert sys.argv[1:]==['serve','--disable-shell','--disable-write']
 assert 'META_API_KEY' not in os.environ
 assert os.environ['MUSE_NO_AUTO_UPDATE']=='1'
session='test-session'
def send(v): print(json.dumps(v,ensure_ascii=False),flush=True)
def note(method,params): send({'jsonrpc':'2.0','method':method,'params':{'sessionId':session,**params}})
for line in sys.stdin:
 q=json.loads(line)
 m=q.get('type') if provider=='pi' else q.get('method')
 p=q if provider=='pi' else q.get('params',{})
 result={}
 if m in ['prompt','turn/start']:
  prompt=p['message'] if provider=='pi' else p['input'][0]['text']
  assert 'Return a JSON object' in prompt
  assert 'feature.rs' in prompt
  if mode=='hang': time.sleep(30)
  if mode=='eof': sys.exit(0)
  if mode=='overflow': print('x'*1048577,flush=True);continue
  if mode=='write': pathlib.Path('unexpected').write_text('no')
 if provider=='pi':
  if m=='set_model':
   assert p['provider']=='test' and p['modelId']=='my/model'
   if mode=='model-error': send({'type':'response','id':q['id'],'command':m,'success':False,'error':'Unknown model'});continue
  if m=='prompt':
   if mode=='tools': send({'type':'tool_execution_start'});continue
   if mode=='error': send({'type':'message_end','message':{'role':'assistant','stopReason':'error','errorMessage':'Usage limit'}});continue
   send({'type':'agent_settled'})
  if m=='get_last_assistant_text':result={'text':'{"subject":"Add Pi support.","body":"Keep   content."}'}
  send({'type':'response','id':q['id'],'command':m,'success':True,'data':result})
 else:
  if m=='initialized':continue
  if m=='initialize':
   assert p['capabilities']['requestedCapabilities']==[]
   result={'schema':{'version':2 if mode=='bad-version' else 1,'fingerprint':'sha256:test'}}
  if m=='session/start':
   assert p['providerId']=='meta' and p['approvalMode']=='denyUnmatched'
   assert p['modelId']=='test/my/model'
   assert pathlib.Path(p['workspaceRoot']).resolve()==pathlib.Path.cwd()
   result={'session':{'sessionId':session}}
   if mode=='bad-ack':result['commandId']='wrong-command'
  if m=='turn/start':
   turn=p['commandId']
   assert turn[14]=='7'
   if mode=='tools':
    note('approval/requested',{'approvalId':'approval','currentRequirementId':'requirement','availableChoices':[{'decision':'denied','choiceId':'deny'}]})
    reply=json.loads(sys.stdin.readline());assert reply['method']=='approval/decide' and reply['params']['choiceId']=='deny'
    continue
   if mode=='input':
    note('userInput/requested',{'userInputId':'input'})
    reply=json.loads(sys.stdin.readline());assert reply['method']=='userInput/cancel';continue
   if mode=='gap':note('view/gap',{});continue
   def item(id,revision,text,**extra): note('item/updated',{'item':{'itemId':id,'revision':revision,'turnId':turn,'kind':'agentMessage','text':text,**extra}})
   item('commentary',1,'Working...')
   item('answer',2,'{"subject":"Add Muse support.","body":"Use native sessions."}',truncated=mode=='truncated')
   item('answer',1,'stale')
   item('retracted',1,'ignore',retracted=True)
   note('item/updated',{'item':{'itemId':'other-turn','revision':99,'turnId':'other','kind':'agentMessage','text':'wrong'}})
   note('turn/completed',{'turnId':turn,'terminal':'failed' if mode=='error' else 'completed','error':{'message':'Usage limit'}})
  send({'jsonrpc':'2.0','id':q['id'],'result':result})
"##.replace("PROVIDER", &json!(if provider == AgentProvider::Pi { "pi" } else { "muse" }).to_string()).replace("MODE", &json!(mode).to_string());
        tokio::fs::write(&script, source).await.unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
        let mut settings = AiSettings::default();
        settings.agent.provider = provider;
        settings.agent.command = Some(script.to_string_lossy().into_owned());
        settings.agent.model = Some("test/my/model".into());
        (directory, settings)
    }
    fn task() -> TextRequest {
        TextRequest {
            system: crate::ai::commit::SYSTEM.into(),
            prompt: "Changed feature.rs".into(),
        }
    }

    #[tokio::test]
    async fn pi_rpc_generates_without_tools_extensions_or_sessions() {
        let (_directory, settings) = fixture(AgentProvider::Pi, "success").await;
        let output = generate(&settings, &task()).await.unwrap();
        assert_eq!(
            crate::ai::commit::decode(&output).unwrap(),
            "Add Pi support\n\nKeep   content."
        );
    }
    #[tokio::test]
    async fn muse_uses_native_read_only_sessions_and_latest_final_message() {
        let (_directory, settings) = fixture(AgentProvider::Muse, "success").await;
        let output = generate(&settings, &task()).await.unwrap();
        assert_eq!(
            crate::ai::commit::decode(&output).unwrap(),
            "Add Muse support\n\nUse native sessions."
        );
    }
    #[tokio::test]
    async fn protocol_errors_and_interaction_never_produce_a_draft() {
        for provider in [AgentProvider::Pi, AgentProvider::Muse] {
            let cases: &[(&str, &str)] = if provider == AgentProvider::Pi {
                &[
                    ("tools", "tools or user input"),
                    ("error", "Usage limit"),
                    ("model-error", "rejected"),
                    ("write", "wrote files"),
                    ("eof", "exited"),
                    ("overflow", "message limit"),
                ]
            } else {
                &[
                    ("tools", "interactive action"),
                    ("input", "interactive action"),
                    ("error", "Usage limit"),
                    ("truncated", "truncated"),
                    ("gap", "lost part"),
                    ("bad-version", "unsupported"),
                    ("bad-ack", "unexpected command ID"),
                    ("write", "wrote files"),
                    ("eof", "exited"),
                    ("overflow", "message limit"),
                ]
            };
            for &(mode, expected) in cases {
                let (_directory, settings) = fixture(provider, mode).await;
                let error = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    generate(&settings, &task()),
                )
                .await
                .unwrap()
                .unwrap_err();
                assert!(
                    format!("{error:#}").contains(expected),
                    "{provider:?} {mode}: {error:#}"
                );
            }
        }
    }
    #[tokio::test]
    async fn invalid_pi_model_and_launch_overrides_fail_clearly() {
        let (_directory, mut settings) = fixture(AgentProvider::Pi, "success").await;
        settings.agent.model = Some("invalid".into());
        assert!(
            generate(&settings, &task())
                .await
                .unwrap_err()
                .to_string()
                .contains("provider/model")
        );
        settings.agent.args = vec!["--extension".into(), "some-script".into()];
        assert!(
            generate(&settings, &task())
                .await
                .unwrap_err()
                .to_string()
                .contains("launch arguments")
        );
    }
    #[tokio::test]
    async fn cancelled_protocol_helpers_release_their_workspace() {
        for provider in [AgentProvider::Pi, AgentProvider::Muse] {
            let (_directory, settings) = fixture(provider, "hang").await;
            let mut process = Process::spawn(&settings).unwrap();
            let workspace = process.workspace.path().to_owned();
            let pid = process.child.id().unwrap() as i32;
            let result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
                if provider == AgentProvider::Pi {
                    process.pi(&settings, &task()).await
                } else {
                    process.muse(&settings, &task()).await
                }
            })
            .await;
            assert!(result.is_err());
            drop(process);
            assert!(!workspace.exists());
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while unsafe { libc::kill(pid, 0) } == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
    }
}
