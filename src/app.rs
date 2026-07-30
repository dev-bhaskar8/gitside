use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use tokio::process::Command;

use crate::{
    config::{Cli, Settings},
    git::GitRepo,
    github::GitHub,
    model::{Branch, Change, Commit, Issue, PullRequest, Remote, RepoStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Commit,
    Changes,
    Staged,
    Graph,
    Branches,
    GitHub,
    Preview,
}

#[derive(Debug, Clone)]
pub enum UiAction {
    Focus(Focus),
    SelectChange { staged: bool, index: usize },
    SelectCommit(usize),
    SelectBranch(usize),
    SelectPullRequest(usize),
    SelectIssue(usize),
    Refresh,
    Fetch,
    Pull,
    Push,
    Commit,
    StageAll,
    UnstageAll,
    ToggleHelp,
    CloseOverlay,
}

#[derive(Debug, Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub action: UiAction,
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub title: String,
    pub body: String,
    pub scroll: u16,
    pub change: Option<Change>,
    pub hunks: Vec<String>,
    pub selected_hunk: usize,
}

#[derive(Debug, Clone)]
pub enum Overlay {
    Help,
    Confirm {
        prompt: String,
        action: ConfirmAction,
    },
    Message {
        title: String,
        body: String,
    },
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    Discard { path: PathBuf, untracked: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    Continue,
    Quit,
    OpenEditor,
}

#[derive(Debug)]
pub struct RepoView {
    pub repo: GitRepo,
    pub status: RepoStatus,
    pub history: Vec<Commit>,
    pub branches: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub github_available: bool,
    pub pull_requests: Vec<PullRequest>,
    pub issues: Vec<Issue>,
}

pub struct App {
    pub settings: Settings,
    pub repos: Vec<RepoView>,
    pub active_repo: usize,
    pub focus: Focus,
    pub selected_change: usize,
    pub selected_staged: usize,
    pub selected_commit: usize,
    pub selected_branch: usize,
    pub selected_github: usize,
    pub github_show_issues: bool,
    pub commit_message: String,
    pub preview: Option<Preview>,
    pub overlay: Option<Overlay>,
    pub output: Vec<String>,
    pub status_line: String,
    pub busy: bool,
    pub hits: Vec<HitRegion>,
    last_click: Option<(u16, u16, Instant)>,
}

impl App {
    pub async fn new(cli: Cli, settings: Settings) -> Result<Self> {
        let mut paths = cli.paths;
        paths.extend(cli.repos);
        if paths.is_empty() {
            paths.push(env::current_dir()?);
        }
        let mut repos = Vec::new();
        for path in paths {
            let repo = GitRepo::discover(&path).await?;
            if repos
                .iter()
                .any(|view: &RepoView| view.repo.root() == repo.root())
            {
                continue;
            }
            let status = repo.status().await?;
            let history = repo.history(settings.graph_page_size).await?;
            let branches = repo.branches().await?;
            let remotes = repo.remotes().await?;
            let github = GitHub::new(repo.root());
            let has_github_remote = remotes.iter().any(|remote| {
                remote.fetch_url.contains("github.com") || remote.push_url.contains("github.com")
            });
            let github_available = has_github_remote && github.available().await;
            repos.push(RepoView {
                repo,
                status,
                history,
                branches,
                remotes,
                github_available,
                pull_requests: Vec::new(),
                issues: Vec::new(),
            });
        }
        if repos.is_empty() {
            bail!("no Git repositories were found");
        }
        Ok(Self {
            settings,
            repos,
            active_repo: 0,
            focus: Focus::Changes,
            selected_change: 0,
            selected_staged: 0,
            selected_commit: 0,
            selected_branch: 0,
            selected_github: 0,
            github_show_issues: false,
            commit_message: String::new(),
            preview: None,
            overlay: None,
            output: Vec::new(),
            status_line: "Ready".into(),
            busy: false,
            hits: Vec::new(),
            last_click: None,
        })
    }

    pub fn active(&self) -> &RepoView {
        &self.repos[self.active_repo]
    }

    pub fn active_mut(&mut self) -> &mut RepoView {
        &mut self.repos[self.active_repo]
    }

    pub fn selected_change(&self) -> Option<&Change> {
        match self.focus {
            Focus::Staged => self.active().status.staged.get(self.selected_staged),
            _ => self.active().status.unstaged.get(self.selected_change),
        }
    }

    pub async fn refresh(&mut self) {
        self.busy = true;
        self.status_line = "Refreshing…".into();
        let repo = self.active().repo.clone();
        let result = async {
            let status = repo.status().await?;
            let history = repo.history(self.settings.graph_page_size).await?;
            let branches = repo.branches().await?;
            let remotes = repo.remotes().await?;
            Ok::<_, anyhow::Error>((status, history, branches, remotes))
        }
        .await;
        match result {
            Ok((status, history, branches, remotes)) => {
                let active = self.active_mut();
                active.status = status;
                active.history = history;
                active.branches = branches;
                active.remotes = remotes;
                self.clamp_selections();
                self.status_line = "Repository refreshed".into();
            }
            Err(error) => self.report_error(error),
        }
        self.busy = false;
    }

    pub async fn handle_key(&mut self, key: KeyEvent) -> EventOutcome {
        if let Some(overlay) = self.overlay.clone() {
            return self.handle_overlay_key(key, overlay).await;
        }
        if self.focus == Focus::Commit {
            match key.code {
                KeyCode::Esc => self.focus = Focus::Changes,
                KeyCode::Backspace => {
                    self.commit_message.pop();
                }
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.commit().await
                }
                KeyCode::Enter => self.commit_message.push('\n'),
                KeyCode::Char(character) => self.commit_message.push(character),
                KeyCode::Tab => self.next_focus(false).await,
                KeyCode::BackTab => self.next_focus(true).await,
                _ => {}
            }
            return EventOutcome::Continue;
        }

        match key.code {
            KeyCode::Char('q') => return EventOutcome::Quit,
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Char('r') => self.refresh().await,
            KeyCode::Char('c') => self.focus = Focus::Commit,
            KeyCode::Char('g') => self.focus = Focus::Graph,
            KeyCode::Char('b') => self.focus = Focus::Branches,
            KeyCode::Char('h') => {
                self.focus = Focus::GitHub;
                self.load_github().await;
            }
            KeyCode::Char('a') => self.run_stage_all().await,
            KeyCode::Char('u') => self.run_unstage_all().await,
            KeyCode::Char('f') => self.run_remote("Fetching", RemoteAction::Fetch).await,
            KeyCode::Char('l') => self.run_remote("Pulling", RemoteAction::Pull).await,
            KeyCode::Char('p') => self.run_remote("Pushing", RemoteAction::Push).await,
            KeyCode::Char('s') => self.run_stash().await,
            KeyCode::Char('e') => {
                if self.editor_change().is_some() {
                    return EventOutcome::OpenEditor;
                }
            }
            KeyCode::Char(']') => self.switch_repo(1).await,
            KeyCode::Char('[') => self.switch_repo(-1).await,
            KeyCode::Char('d') => self.request_discard(),
            KeyCode::Char('i') if self.focus == Focus::GitHub => {
                self.github_show_issues = !self.github_show_issues;
                self.selected_github = 0;
            }
            KeyCode::Tab => self.next_focus(false).await,
            KeyCode::BackTab => self.next_focus(true).await,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home => self.set_selection(0),
            KeyCode::End => self.set_selection(usize::MAX),
            KeyCode::Enter => self.activate().await,
            KeyCode::Char(' ') => self.toggle_stage().await,
            KeyCode::Esc => {
                self.preview = None;
                if self.focus == Focus::Preview {
                    self.focus = Focus::Changes;
                }
            }
            _ => {}
        }
        EventOutcome::Continue
    }

    async fn handle_overlay_key(&mut self, key: KeyEvent, overlay: Overlay) -> EventOutcome {
        match overlay {
            Overlay::Confirm { action, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.overlay = None;
                    self.execute_confirmed(action).await;
                }
                KeyCode::Char('n') | KeyCode::Esc => self.overlay = None,
                _ => {}
            },
            _ => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?')) {
                    self.overlay = None;
                }
            }
        }
        EventOutcome::Continue
    }

    pub async fn handle_mouse(&mut self, event: MouseEvent) -> EventOutcome {
        match event.kind {
            MouseEventKind::ScrollUp => {
                self.move_selection(-3);
                return EventOutcome::Continue;
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(3);
                return EventOutcome::Continue;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let now = Instant::now();
                let double = self.last_click.is_some_and(|(x, y, when)| {
                    x == event.column
                        && y == event.row
                        && now.duration_since(when) < Duration::from_millis(450)
                });
                self.last_click = Some((event.column, event.row, now));
                if let Some(action) = self
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| hit.rect.contains((event.column, event.row).into()))
                    .map(|hit| hit.action.clone())
                {
                    self.perform_ui_action(action).await;
                    if double {
                        self.activate().await;
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) if self.selected_change().is_some() => {
                self.overlay = Some(Overlay::Message {
                    title: "Change actions".into(),
                    body: "Space  Stage/unstage\nEnter  Preview\ne  Open in editor\nd  Discard\nEsc  Close".into(),
                });
            }
            _ => {}
        }
        EventOutcome::Continue
    }

    async fn perform_ui_action(&mut self, action: UiAction) {
        match action {
            UiAction::Focus(focus) => self.focus = focus,
            UiAction::SelectChange { staged, index } => {
                if staged {
                    self.focus = Focus::Staged;
                    self.selected_staged = index;
                } else {
                    self.focus = Focus::Changes;
                    self.selected_change = index;
                }
            }
            UiAction::SelectCommit(index) => {
                self.focus = Focus::Graph;
                self.selected_commit = index;
            }
            UiAction::SelectBranch(index) => {
                self.focus = Focus::Branches;
                self.selected_branch = index;
            }
            UiAction::SelectPullRequest(index) | UiAction::SelectIssue(index) => {
                self.focus = Focus::GitHub;
                self.selected_github = index;
            }
            UiAction::Refresh => self.refresh().await,
            UiAction::Fetch => self.run_remote("Fetching", RemoteAction::Fetch).await,
            UiAction::Pull => self.run_remote("Pulling", RemoteAction::Pull).await,
            UiAction::Push => self.run_remote("Pushing", RemoteAction::Push).await,
            UiAction::Commit => self.commit().await,
            UiAction::StageAll => self.run_stage_all().await,
            UiAction::UnstageAll => self.run_unstage_all().await,
            UiAction::ToggleHelp => self.overlay = Some(Overlay::Help),
            UiAction::CloseOverlay => self.overlay = None,
        }
    }

    async fn activate(&mut self) {
        match self.focus {
            Focus::Changes | Focus::Staged => self.open_change_preview().await,
            Focus::Graph => self.open_commit_preview().await,
            Focus::Branches => self.checkout_selected().await,
            Focus::Preview => {}
            _ => {}
        }
    }

    async fn open_change_preview(&mut self) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        match repo.diff(&change).await {
            Ok(body) => {
                self.preview = Some(Preview {
                    title: change.path.display().to_string(),
                    hunks: split_diff_hunks(&body),
                    body,
                    scroll: 0,
                    change: Some(change),
                    selected_hunk: 0,
                });
                self.focus = Focus::Preview;
            }
            Err(error) => self.report_error(error),
        }
    }

    async fn open_commit_preview(&mut self) {
        let Some(commit) = self.active().history.get(self.selected_commit).cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        match repo.show_commit(&commit.oid).await {
            Ok(body) => {
                self.preview = Some(Preview {
                    title: format!("{} {}", short_oid(&commit.oid), commit.subject),
                    body,
                    scroll: 0,
                    change: None,
                    hunks: Vec::new(),
                    selected_hunk: 0,
                });
                self.focus = Focus::Preview;
            }
            Err(error) => self.report_error(error),
        }
    }

    async fn toggle_stage(&mut self) {
        if self.focus == Focus::Preview {
            self.toggle_preview_hunk().await;
            return;
        }
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        let result = if change.staged {
            repo.unstage(&change.path).await
        } else {
            repo.stage(&change.path).await
        };
        self.finish_action(result, if change.staged { "Unstaged" } else { "Staged" })
            .await;
    }

    async fn run_stage_all(&mut self) {
        let repo = self.active().repo.clone();
        let result = repo.stage_all().await;
        self.finish_action(result, "Staged all changes").await;
    }

    async fn run_unstage_all(&mut self) {
        let repo = self.active().repo.clone();
        let result = repo.unstage_all().await;
        self.finish_action(result, "Unstaged all changes").await;
    }

    async fn commit(&mut self) {
        let repo = self.active().repo.clone();
        let result = repo.commit(&self.commit_message, false, false).await;
        if result.is_ok() {
            self.commit_message.clear();
            self.focus = Focus::Changes;
        }
        self.finish_action(result, "Committed changes").await;
    }

    async fn run_remote(&mut self, label: &str, action: RemoteAction) {
        self.status_line = format!("{label}…");
        let repo = self.active().repo.clone();
        let result = match action {
            RemoteAction::Fetch => repo.fetch().await,
            RemoteAction::Pull => repo.pull().await,
            RemoteAction::Push => repo.push().await,
        };
        self.finish_action(result, &format!("{label} complete"))
            .await;
    }

    async fn run_stash(&mut self) {
        let repo = self.active().repo.clone();
        let result = repo.stash().await;
        self.finish_action(result, "Created stash").await;
    }

    async fn checkout_selected(&mut self) {
        let Some(branch) = self.active().branches.get(self.selected_branch).cloned() else {
            return;
        };
        if branch.current {
            self.status_line = format!("Already on {}", branch.name);
            return;
        }
        let repo = self.active().repo.clone();
        let result = repo.checkout(&branch.name).await;
        self.finish_action(result, &format!("Switched to {}", branch.name))
            .await;
    }

    fn request_discard(&mut self) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        if change.staged {
            self.status_line = "Unstage before discarding a staged change".into();
            return;
        }
        let action = ConfirmAction::Discard {
            path: change.path.clone(),
            untracked: matches!(change.kind, crate::model::ChangeKind::Untracked),
        };
        if self.settings.confirm_destructive {
            self.overlay = Some(Overlay::Confirm {
                prompt: format!(
                    "Discard all working-tree changes in {}?\nThis cannot be undone. [y/N]",
                    change.path.display()
                ),
                action,
            });
        }
    }

    async fn execute_confirmed(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Discard { path, untracked } => {
                let repo = self.active().repo.clone();
                let result = repo.discard(&path, untracked).await;
                self.finish_action(result, "Discarded change").await;
            }
        }
    }

    async fn finish_action(&mut self, result: Result<()>, success: &str) {
        match result {
            Ok(()) => {
                self.status_line = success.into();
                self.refresh().await;
                self.status_line = success.into();
            }
            Err(error) => self.report_error(error),
        }
    }

    async fn load_github(&mut self) {
        if !self.active().github_available {
            self.status_line = "GitHub CLI unavailable or repository is not authenticated".into();
            return;
        }
        if !self.active().pull_requests.is_empty() || !self.active().issues.is_empty() {
            return;
        }
        self.status_line = "Loading GitHub…".into();
        let github = GitHub::new(self.active().repo.root());
        let (prs, issues) = tokio::join!(github.pull_requests(), github.issues());
        match (prs, issues) {
            (Ok(prs), Ok(issues)) => {
                let active = self.active_mut();
                active.pull_requests = prs;
                active.issues = issues;
                self.status_line = "GitHub data loaded".into();
            }
            (Err(error), _) | (_, Err(error)) => self.report_error(error),
        }
    }

    pub async fn open_selected_in_editor(&mut self) -> Result<()> {
        let change = self
            .editor_change()
            .cloned()
            .context("no changed file selected")?;
        let absolute = self.active().repo.root().join(change.path);
        let (program, mut args) = self.resolve_editor()?;
        for arg in &mut args {
            *arg = arg.replace("{path}", &absolute.to_string_lossy());
        }
        if !args
            .iter()
            .any(|arg| arg.contains(absolute.to_string_lossy().as_ref()))
        {
            args.push(absolute.to_string_lossy().into_owned());
        }
        let status = Command::new(&program)
            .args(args)
            .current_dir(self.active().repo.root())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| format!("failed to open editor {program}"))?;
        if !status.success() {
            bail!("editor exited with {}", status.code().unwrap_or(-1));
        }
        Ok(())
    }

    fn resolve_editor(&self) -> Result<(String, Vec<String>)> {
        if let Some(command) = &self.settings.editor.command {
            let parsed = shell_words::split(command)?;
            if let Some((program, command_args)) = parsed.split_first() {
                let mut args = command_args.to_vec();
                args.extend(self.settings.editor.args.clone());
                return Ok((program.clone(), args));
            }
        }
        for variable in ["VISUAL", "EDITOR"] {
            if let Ok(command) = env::var(variable) {
                let parsed = shell_words::split(&command)?;
                if let Some((program, args)) = parsed.split_first() {
                    return Ok((program.clone(), args.to_vec()));
                }
            }
        }
        for candidate in editor_candidates() {
            if executable_available(candidate).is_some() {
                return Ok(((*candidate).into(), Vec::new()));
            }
        }
        bail!("no editor detected; configure [editor].command or set VISUAL/EDITOR")
    }

    async fn next_focus(&mut self, backwards: bool) {
        const ORDER: [Focus; 6] = [
            Focus::Commit,
            Focus::Changes,
            Focus::Staged,
            Focus::Graph,
            Focus::Branches,
            Focus::GitHub,
        ];
        let current = ORDER
            .iter()
            .position(|value| *value == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(ORDER.len() - 1)
        } else {
            (current + 1) % ORDER.len()
        };
        self.focus = ORDER[next];
        if self.focus == Focus::GitHub {
            self.load_github().await;
        }
    }

    fn move_selection(&mut self, amount: isize) {
        if self.focus == Focus::Preview {
            if let Some(preview) = &mut self.preview {
                if preview.hunks.is_empty() {
                    preview.scroll = if amount.is_negative() {
                        preview.scroll.saturating_sub(amount.unsigned_abs() as u16)
                    } else {
                        preview.scroll.saturating_add(amount as u16)
                    };
                } else {
                    let max = preview.hunks.len().saturating_sub(1);
                    preview.selected_hunk = if amount.is_negative() {
                        preview.selected_hunk.saturating_sub(amount.unsigned_abs())
                    } else {
                        preview
                            .selected_hunk
                            .saturating_add(amount as usize)
                            .min(max)
                    };
                    preview.scroll = hunk_line_offset(&preview.body, preview.selected_hunk);
                }
            }
            return;
        }
        let current = self.current_selection();
        let max = self.current_len().saturating_sub(1);
        let value = if amount.is_negative() {
            current.saturating_sub(amount.unsigned_abs())
        } else {
            current.saturating_add(amount as usize).min(max)
        };
        self.set_selection(value);
    }

    fn current_selection(&self) -> usize {
        match self.focus {
            Focus::Staged => self.selected_staged,
            Focus::Graph => self.selected_commit,
            Focus::Branches => self.selected_branch,
            Focus::GitHub => self.selected_github,
            _ => self.selected_change,
        }
    }

    fn current_len(&self) -> usize {
        match self.focus {
            Focus::Staged => self.active().status.staged.len(),
            Focus::Graph => self.active().history.len(),
            Focus::Branches => self.active().branches.len(),
            Focus::GitHub if self.github_show_issues => self.active().issues.len(),
            Focus::GitHub => self.active().pull_requests.len(),
            _ => self.active().status.unstaged.len(),
        }
    }

    fn set_selection(&mut self, value: usize) {
        let clamped = value.min(self.current_len().saturating_sub(1));
        match self.focus {
            Focus::Staged => self.selected_staged = clamped,
            Focus::Graph => self.selected_commit = clamped,
            Focus::Branches => self.selected_branch = clamped,
            Focus::GitHub => self.selected_github = clamped,
            _ => self.selected_change = clamped,
        }
    }

    fn clamp_selections(&mut self) {
        self.selected_change = self
            .selected_change
            .min(self.active().status.unstaged.len().saturating_sub(1));
        self.selected_staged = self
            .selected_staged
            .min(self.active().status.staged.len().saturating_sub(1));
        self.selected_commit = self
            .selected_commit
            .min(self.active().history.len().saturating_sub(1));
        self.selected_branch = self
            .selected_branch
            .min(self.active().branches.len().saturating_sub(1));
    }

    fn report_error(&mut self, error: anyhow::Error) {
        let message = format!("{error:#}");
        self.output.push(message.clone());
        self.status_line = message.clone();
        self.overlay = Some(Overlay::Message {
            title: "Command failed".into(),
            body: message,
        });
    }

    fn editor_change(&self) -> Option<&Change> {
        self.preview
            .as_ref()
            .and_then(|preview| preview.change.as_ref())
            .or_else(|| self.selected_change())
    }

    async fn toggle_preview_hunk(&mut self) {
        let Some(preview) = self.preview.as_ref() else {
            return;
        };
        let Some(change) = preview.change.clone() else {
            self.status_line = "Commit previews cannot be staged".into();
            return;
        };
        let Some(patch) = preview.hunks.get(preview.selected_hunk).cloned() else {
            self.status_line = "This diff has no independently stageable text hunks".into();
            return;
        };
        let repo = self.active().repo.clone();
        let result = repo.apply_cached_patch(&patch, change.staged).await;
        if result.is_ok() {
            self.preview = None;
            self.focus = if change.staged {
                Focus::Staged
            } else {
                Focus::Changes
            };
        }
        self.finish_action(
            result,
            if change.staged {
                "Unstaged hunk"
            } else {
                "Staged hunk"
            },
        )
        .await;
    }

    async fn switch_repo(&mut self, amount: isize) {
        if self.repos.len() < 2 {
            self.status_line = "Only one repository is open".into();
            return;
        }
        self.active_repo = if amount.is_negative() {
            self.active_repo
                .checked_sub(1)
                .unwrap_or(self.repos.len() - 1)
        } else {
            (self.active_repo + 1) % self.repos.len()
        };
        self.preview = None;
        self.focus = Focus::Changes;
        self.clamp_selections();
        self.status_line = format!("Opened {}", self.active().repo.name());
    }
}

#[derive(Debug, Clone, Copy)]
enum RemoteAction {
    Fetch,
    Pull,
    Push,
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

fn editor_candidates() -> &'static [&'static str] {
    if cfg!(windows) {
        &["code.cmd", "cursor.cmd", "notepad.exe"]
    } else {
        &["code", "cursor", "nvim", "vim", "nano"]
    }
}

fn executable_available(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 && path.is_file() {
        return Some(path.into());
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|directory| {
            let candidate = directory.join(program);
            candidate.is_file().then_some(candidate)
        })
    })
}

fn split_diff_hunks(diff: &str) -> Vec<String> {
    let lines: Vec<&str> = diff.split_inclusive('\n').collect();
    let Some(first_hunk) = lines.iter().position(|line| line.starts_with("@@")) else {
        return Vec::new();
    };
    let header: String = lines[..first_hunk].concat();
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with("@@").then_some(index))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(lines.len());
            format!("{}{}", header, lines[*start..end].concat())
        })
        .collect()
}

fn hunk_line_offset(diff: &str, selected_hunk: usize) -> u16 {
    diff.lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("@@"))
        .nth(selected_hunk)
        .map(|(index, _)| index.saturating_sub(2) as u16)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_patch_into_independently_applicable_hunks() {
        let diff =
            "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n@@ -9 +9 @@\n-x\n+y\n";
        let hunks = split_diff_hunks(diff);
        assert_eq!(hunks.len(), 2);
        assert!(hunks[0].contains("-old"));
        assert!(!hunks[0].contains("-x"));
        assert!(hunks[1].starts_with("diff --git"));
    }
}
