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
use tokio::task::JoinHandle;

use crate::{
    config::{Cli, Settings},
    git::{CommitOptions, ConflictChoice, GitRepo},
    github::{GitHub, GitHubConnectionState, GitHubVisibility},
    model::{Branch, Change, Commit, Issue, PullRequest, Remote, RepoStatus, Stash, Worktree},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Commit,
    Changes,
    Staged,
    Graph,
    Branches,
    Stashes,
    Worktrees,
    GitHub,
    Preview,
}

#[derive(Debug, Clone)]
pub enum UiAction {
    Focus(Focus),
    SelectChange { staged: bool, index: usize },
    SelectCommit(usize),
    SelectBranch(usize),
    SelectStash(usize),
    SelectWorktree(usize),
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
    PublishGitHub,
    ConfirmGitHubVisibility(GitHubVisibility),
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
    Help {
        scroll: u16,
        max_scroll: u16,
    },
    Confirm {
        prompt: String,
        action: ConfirmAction,
    },
    Message {
        title: String,
        body: String,
    },
    Prompt {
        title: String,
        label: String,
        value: String,
        action: PromptAction,
        replace_on_type: bool,
    },
    GitHubVisibility {
        name: String,
        selected: GitHubVisibility,
    },
    Search {
        value: String,
    },
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    Discard {
        path: PathBuf,
        untracked: bool,
    },
    DeleteBranch {
        name: String,
    },
    Rebase {
        branch: String,
    },
    DropStash {
        reference: String,
    },
    RemoveWorktree {
        path: PathBuf,
    },
    UndoLastCommit,
    ForcePush {
        remote: String,
        branch: String,
    },
    PublishGitHub {
        name: String,
        visibility: GitHubVisibility,
        remote: String,
        push: bool,
    },
}

#[derive(Debug, Clone)]
pub enum PromptAction {
    CreateBranch,
    CreateTag { oid: String },
    AddWorktree { branch: String },
    PushTarget { remote: String, branch: String },
    PullTarget { remote: String, branch: String },
    PublishGitHubName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    Continue,
    Quit,
    OpenDifftool,
    InteractiveStage,
    OpenEditor,
}

#[derive(Debug)]
pub struct RepoView {
    pub repo: GitRepo,
    pub status: RepoStatus,
    pub history: Vec<Commit>,
    pub branches: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub stashes: Vec<Stash>,
    pub worktrees: Vec<Worktree>,
    pub github_state: GitHubConnectionState,
    github_loaded: bool,
    pub pull_requests: Vec<PullRequest>,
    pub issues: Vec<Issue>,
    history_limit: usize,
    history_exhausted: bool,
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
    pub selected_stash: usize,
    pub selected_worktree: usize,
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
    background_task: Option<JoinHandle<BackgroundResult>>,
    last_search: Option<String>,
    commit_history_index: Option<usize>,
    commit_history_draft: String,
}

enum BackgroundResult {
    Remote {
        repo_index: usize,
        label: &'static str,
        result: Result<()>,
    },
    History {
        repo_index: usize,
        requested_limit: usize,
        result: Result<Vec<Commit>>,
    },
    Refresh {
        repo_index: usize,
        show_status: bool,
        result: Box<Result<RepoSnapshot>>,
    },
    GitHub {
        repo_index: usize,
        result: Result<(Vec<PullRequest>, Vec<Issue>)>,
    },
    PublishGitHub {
        repo_index: usize,
        result: Result<()>,
    },
}

struct RepoSnapshot {
    status: RepoStatus,
    history: Vec<Commit>,
    branches: Vec<Branch>,
    remotes: Vec<Remote>,
    stashes: Vec<Stash>,
    worktrees: Vec<Worktree>,
    github_state: GitHubConnectionState,
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
            let snapshot = load_repo_snapshot(repo.clone(), settings.graph_page_size).await?;
            let history_exhausted = snapshot.history.len() < settings.graph_page_size;
            repos.push(RepoView {
                repo,
                status: snapshot.status,
                history: snapshot.history,
                branches: snapshot.branches,
                remotes: snapshot.remotes,
                stashes: snapshot.stashes,
                worktrees: snapshot.worktrees,
                github_state: snapshot.github_state,
                github_loaded: false,
                pull_requests: Vec::new(),
                issues: Vec::new(),
                history_limit: settings.graph_page_size,
                history_exhausted,
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
            selected_stash: 0,
            selected_worktree: 0,
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
            background_task: None,
            last_search: None,
            commit_history_index: None,
            commit_history_draft: String::new(),
        })
    }

    pub fn active(&self) -> &RepoView {
        &self.repos[self.active_repo]
    }

    #[cfg(test)]
    pub fn active_mut(&mut self) -> &mut RepoView {
        &mut self.repos[self.active_repo]
    }

    pub fn selected_change(&self) -> Option<&Change> {
        match self.focus {
            Focus::Staged => self.active().status.staged.get(self.selected_staged),
            Focus::Changes if !self.active().status.conflicts.is_empty() => {
                self.active().status.conflicts.get(self.selected_change)
            }
            _ => self.active().status.unstaged.get(self.selected_change),
        }
    }

    pub async fn refresh(&mut self) {
        self.refresh_repo(self.active_repo, true).await;
    }

    async fn refresh_repo(&mut self, repo_index: usize, show_status: bool) {
        self.busy = true;
        if show_status {
            self.status_line = "Refreshing…".into();
        }
        let Some(view) = self.repos.get(repo_index) else {
            self.busy = false;
            return;
        };
        let repo = view.repo.clone();
        let history_limit = view.history_limit;
        let result = load_repo_snapshot(repo, history_limit).await;
        match result {
            Ok(snapshot) => {
                self.apply_snapshot(repo_index, history_limit, snapshot);
                if show_status {
                    self.status_line = "Repository refreshed".into();
                }
            }
            Err(error) => self.report_error(error),
        }
        self.busy = false;
    }

    pub fn queue_refresh(&mut self, show_status: bool) {
        if self.background_task.is_some() {
            return;
        }
        let repo_index = self.active_repo;
        let view = self.active();
        let repo = view.repo.clone();
        let history_limit = view.history_limit;
        self.busy = true;
        if show_status {
            self.status_line = "Refreshing…".into();
        }
        self.background_task = Some(tokio::spawn(async move {
            BackgroundResult::Refresh {
                repo_index,
                show_status,
                result: Box::new(load_repo_snapshot(repo, history_limit).await),
            }
        }));
    }

    fn apply_snapshot(&mut self, repo_index: usize, history_limit: usize, snapshot: RepoSnapshot) {
        if let Some(view) = self.repos.get_mut(repo_index) {
            view.status = snapshot.status;
            view.history_exhausted = snapshot.history.len() < history_limit;
            view.history = snapshot.history;
            view.branches = snapshot.branches;
            view.remotes = snapshot.remotes;
            view.stashes = snapshot.stashes;
            view.worktrees = snapshot.worktrees;
            view.github_state = snapshot.github_state;
            if view.github_state != GitHubConnectionState::Ready {
                view.github_loaded = false;
                view.pull_requests.clear();
                view.issues.clear();
            }
        }
        if repo_index == self.active_repo {
            self.clamp_selections();
        }
    }

    pub async fn poll_background(&mut self) -> bool {
        if !self
            .background_task
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            return false;
        }
        let Some(task) = self.background_task.take() else {
            return false;
        };
        self.busy = false;
        match task.await {
            Ok(BackgroundResult::Remote {
                repo_index,
                label,
                result,
            }) => match result {
                Ok(()) => {
                    self.refresh_repo(repo_index, false).await;
                    self.status_line = format!("{label} complete");
                }
                Err(error) => self.report_error(error),
            },
            Ok(BackgroundResult::History {
                repo_index,
                requested_limit,
                result,
            }) => match result {
                Ok(history) => {
                    if let Some(view) = self.repos.get_mut(repo_index) {
                        view.history_exhausted = history.len() < requested_limit;
                        view.history_limit = requested_limit;
                        view.history = history;
                    }
                    if repo_index == self.active_repo {
                        self.clamp_selections();
                    }
                    self.status_line = "Loaded more history".into();
                }
                Err(error) => self.report_error(error),
            },
            Ok(BackgroundResult::Refresh {
                repo_index,
                show_status,
                result,
            }) => match *result {
                Ok(snapshot) => {
                    let history_limit = self
                        .repos
                        .get(repo_index)
                        .map(|view| view.history_limit)
                        .unwrap_or(self.settings.graph_page_size);
                    self.apply_snapshot(repo_index, history_limit, snapshot);
                    if show_status {
                        self.status_line = "Repository refreshed".into();
                    }
                    if repo_index == self.active_repo
                        && self.focus == Focus::GitHub
                        && self.active().github_state == GitHubConnectionState::Ready
                        && !self.active().github_loaded
                    {
                        self.load_github();
                    }
                }
                Err(error) => self.report_error(error),
            },
            Ok(BackgroundResult::GitHub { repo_index, result }) => match result {
                Ok((pull_requests, issues)) => {
                    if let Some(view) = self.repos.get_mut(repo_index) {
                        view.pull_requests = pull_requests;
                        view.issues = issues;
                        view.github_loaded = true;
                    }
                    self.status_line = "Loaded GitHub data".into();
                }
                Err(error) => self.report_error(error),
            },
            Ok(BackgroundResult::PublishGitHub { repo_index, result }) => match result {
                Ok(()) => {
                    self.refresh_repo(repo_index, false).await;
                    self.status_line = "Published repository to GitHub".into();
                    if repo_index == self.active_repo && self.focus == Focus::GitHub {
                        self.load_github();
                    }
                }
                Err(error) => self.report_error(error),
            },
            Err(error) => self.report_error(anyhow::anyhow!("background Git task failed: {error}")),
        }
        true
    }

    pub async fn handle_key(&mut self, key: KeyEvent) -> EventOutcome {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return EventOutcome::Quit;
        }
        if let Some(overlay) = self.overlay.clone() {
            return self.handle_overlay_key(key, overlay).await;
        }
        if key.code == KeyCode::F(1) {
            self.open_help();
            return EventOutcome::Continue;
        }
        if self.focus == Focus::Commit {
            if let Some(options) = commit_options_for_key(key) {
                self.commit(options).await;
                return EventOutcome::Continue;
            }
            match key.code {
                KeyCode::Esc => self.focus = Focus::Changes,
                KeyCode::Backspace => {
                    self.commit_message.pop();
                    self.commit_history_index = None;
                }
                KeyCode::Enter => self.commit_message.push('\n'),
                KeyCode::Up => self.recall_commit_message(-1),
                KeyCode::Down => self.recall_commit_message(1),
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.commit_message.push(character);
                    self.commit_history_index = None;
                }
                KeyCode::Tab => self.next_focus(false).await,
                KeyCode::BackTab => self.next_focus(true).await,
                _ => {}
            }
            return EventOutcome::Continue;
        }

        match key.code {
            KeyCode::Char('q') => return EventOutcome::Quit,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('/') => {
                self.overlay = Some(Overlay::Search {
                    value: String::new(),
                })
            }
            KeyCode::Char('r') => self.queue_refresh(true),
            KeyCode::Char('O') if self.conflicts_focused() => {
                self.resolve_selected_conflict(ConflictChoice::Current)
                    .await
            }
            KeyCode::Char('I') if self.conflicts_focused() => {
                self.resolve_selected_conflict(ConflictChoice::Incoming)
                    .await
            }
            KeyCode::Char('B') if self.conflicts_focused() => {
                self.resolve_selected_conflict(ConflictChoice::Both).await
            }
            KeyCode::Char('C') if self.operation_focused() => self.continue_operation().await,
            KeyCode::Char('A') if self.operation_focused() => self.abort_operation().await,
            KeyCode::Char('S') if self.operation_focused() => self.skip_operation().await,
            KeyCode::Char('c') => self.focus = Focus::Commit,
            KeyCode::Char('g') => self.focus = Focus::Graph,
            KeyCode::Char('b') => self.focus = Focus::Branches,
            KeyCode::Char('h') => self.focus_github(),
            KeyCode::Char('a') => self.run_stage_all().await,
            KeyCode::Char('u') => self.run_unstage_all().await,
            KeyCode::Char('f') => self.run_remote("Fetching", RemoteAction::Fetch),
            KeyCode::Char('l') => self.run_remote("Pulling", RemoteAction::Pull),
            KeyCode::Char('L') => self.run_remote("Pulling with rebase", RemoteAction::PullRebase),
            KeyCode::Char('p') => self.run_contextual_push(),
            KeyCode::Char('P') if self.focus != Focus::Stashes => self.open_remote_prompt(true),
            KeyCode::Char('T') => self.open_remote_prompt(false),
            KeyCode::Char('F') => self.request_force_push().await,
            KeyCode::Char('U') => self.request_undo_last_commit().await,
            KeyCode::Char('D') => self.open_diagnostics(),
            KeyCode::Char('s') => self.run_stash().await,
            KeyCode::Char('z') => self.focus = Focus::Stashes,
            KeyCode::Char('W') => self.focus = Focus::Worktrees,
            KeyCode::Char('w') if self.focus == Focus::Branches => {
                if let Some(branch) = self.active().branches.get(self.selected_branch) {
                    self.overlay = Some(Overlay::Prompt {
                        title: "Add worktree".into(),
                        label: format!("Path for branch {}", branch.name),
                        value: String::new(),
                        replace_on_type: false,
                        action: PromptAction::AddWorktree {
                            branch: branch.name.clone(),
                        },
                    });
                }
            }
            KeyCode::Char('n') if self.focus == Focus::Branches => {
                self.overlay = Some(Overlay::Prompt {
                    title: "Create branch".into(),
                    label: "Branch name".into(),
                    value: String::new(),
                    replace_on_type: false,
                    action: PromptAction::CreateBranch,
                });
            }
            KeyCode::Char('N') => self.repeat_search(),
            KeyCode::Char('x') if self.focus == Focus::Branches => {
                self.request_delete_branch().await
            }
            KeyCode::Char('m') if self.focus == Focus::Branches => self.merge_selected().await,
            KeyCode::Char('R') if self.focus == Focus::Branches => self.request_rebase().await,
            KeyCode::Char('y') if self.focus == Focus::Graph => self.cherry_pick_selected().await,
            KeyCode::Char('v') if self.focus == Focus::Graph => self.revert_selected().await,
            KeyCode::Char('t') if self.focus == Focus::Graph => {
                if let Some(commit) = self.active().history.get(self.selected_commit) {
                    self.overlay = Some(Overlay::Prompt {
                        title: "Create tag".into(),
                        label: "Tag name".into(),
                        value: String::new(),
                        replace_on_type: false,
                        action: PromptAction::CreateTag {
                            oid: commit.oid.clone(),
                        },
                    });
                }
            }
            KeyCode::Char('e') => {
                if let Some(change) = self.editor_change() {
                    return if matches!(change.kind, crate::model::ChangeKind::Untracked) {
                        EventOutcome::OpenEditor
                    } else {
                        EventOutcome::OpenDifftool
                    };
                }
            }
            KeyCode::Char('E') => {
                if self.editor_change().is_some() {
                    return EventOutcome::InteractiveStage;
                }
            }
            KeyCode::Char('o') if self.focus != Focus::GitHub => {
                if self.editor_change().is_some() {
                    return EventOutcome::OpenEditor;
                }
            }
            KeyCode::Char(']') => self.switch_repo(1).await,
            KeyCode::Char('[') => self.switch_repo(-1).await,
            KeyCode::Char('d') => self.request_discard().await,
            KeyCode::Char('i') if self.focus == Focus::GitHub => {
                self.github_show_issues = !self.github_show_issues;
                self.selected_github = 0;
            }
            KeyCode::Char('o') if self.focus == Focus::GitHub => {
                self.open_github_in_browser().await
            }
            KeyCode::Char('C') if self.focus == Focus::GitHub && !self.github_show_issues => {
                self.checkout_pull_request().await
            }
            KeyCode::Char('K') if self.focus == Focus::GitHub && !self.github_show_issues => {
                self.open_pull_request_checks().await
            }
            KeyCode::Char('A') if self.focus == Focus::Stashes => {
                self.apply_selected_stash(false).await
            }
            KeyCode::Char('P') if self.focus == Focus::Stashes => {
                self.apply_selected_stash(true).await
            }
            KeyCode::Char('X') if self.focus == Focus::Stashes => self.request_drop_stash().await,
            KeyCode::Char('X') if self.focus == Focus::Worktrees => {
                self.request_remove_worktree().await
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
            Overlay::Help {
                mut scroll,
                max_scroll,
            } => {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::F(1) => {
                        self.overlay = None;
                        return EventOutcome::Continue;
                    }
                    KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        scroll = scroll.saturating_add(1).min(max_scroll)
                    }
                    KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                    KeyCode::PageDown => scroll = scroll.saturating_add(10).min(max_scroll),
                    KeyCode::Home => scroll = 0,
                    KeyCode::End => scroll = max_scroll,
                    _ => {}
                }
                self.overlay = Some(Overlay::Help { scroll, max_scroll });
            }
            Overlay::Confirm { action, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.overlay = None;
                    self.execute_confirmed(action).await;
                }
                KeyCode::Char('n') | KeyCode::Esc => self.overlay = None,
                _ => {}
            },
            Overlay::Prompt { action, .. } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Backspace => {
                    if let Some(Overlay::Prompt {
                        value,
                        replace_on_type,
                        ..
                    }) = &mut self.overlay
                    {
                        if *replace_on_type {
                            value.clear();
                            *replace_on_type = false;
                        } else {
                            value.pop();
                        }
                    }
                }
                KeyCode::Enter => {
                    let value = match self.overlay.take() {
                        Some(Overlay::Prompt { value, .. }) => value,
                        _ => String::new(),
                    };
                    self.execute_prompt(action, value).await;
                }
                KeyCode::Char(character) => {
                    if let Some(Overlay::Prompt {
                        value,
                        replace_on_type,
                        ..
                    }) = &mut self.overlay
                    {
                        if *replace_on_type {
                            value.clear();
                            *replace_on_type = false;
                        }
                        value.push(character);
                    }
                }
                _ => {}
            },
            Overlay::GitHubVisibility { name, mut selected } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                    selected = match selected {
                        GitHubVisibility::Private => GitHubVisibility::Public,
                        GitHubVisibility::Public => GitHubVisibility::Private,
                    };
                    self.overlay = Some(Overlay::GitHubVisibility { name, selected });
                }
                KeyCode::Char('p') => {
                    self.overlay = Some(Overlay::GitHubVisibility {
                        name,
                        selected: GitHubVisibility::Private,
                    })
                }
                KeyCode::Char('u') => {
                    self.overlay = Some(Overlay::GitHubVisibility {
                        name,
                        selected: GitHubVisibility::Public,
                    })
                }
                KeyCode::Enter => {
                    self.overlay = None;
                    self.request_publish_github(name, selected).await;
                }
                _ => {}
            },
            Overlay::Search { .. } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Backspace => {
                    if let Some(Overlay::Search { value }) = &mut self.overlay {
                        value.pop();
                    }
                }
                KeyCode::Enter => {
                    let value = match self.overlay.take() {
                        Some(Overlay::Search { value }) => value,
                        _ => String::new(),
                    };
                    self.search_focused(value, false);
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if let Some(Overlay::Search { value }) = &mut self.overlay {
                        value.push(character);
                    }
                }
                _ => {}
            },
            _ => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::F(1)
                ) {
                    self.overlay = None;
                }
            }
        }
        EventOutcome::Continue
    }

    pub async fn handle_mouse(&mut self, event: MouseEvent) -> EventOutcome {
        if let Some(Overlay::Help { scroll, max_scroll }) = &mut self.overlay {
            match event.kind {
                MouseEventKind::ScrollUp => *scroll = scroll.saturating_sub(3),
                MouseEventKind::ScrollDown => *scroll = scroll.saturating_add(3).min(*max_scroll),
                _ => {}
            }
            if matches!(
                event.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) {
                return EventOutcome::Continue;
            }
        } else if self.overlay.is_some()
            && matches!(
                event.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            return EventOutcome::Continue;
        }

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
                    let activate_on_double_click = matches!(
                        action,
                        UiAction::SelectChange { .. }
                            | UiAction::SelectCommit(_)
                            | UiAction::SelectBranch(_)
                            | UiAction::SelectStash(_)
                            | UiAction::SelectWorktree(_)
                            | UiAction::SelectPullRequest(_)
                            | UiAction::SelectIssue(_)
                    );
                    self.perform_ui_action(action).await;
                    if double && activate_on_double_click {
                        self.activate().await;
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) if self.selected_change().is_some() => {
                self.overlay = Some(Overlay::Message {
                    title: "Change actions".into(),
                    body: "Space  Stage/unstage\nEnter  Preview\ne  External diff\nE  Select lines\nd  Discard\nEsc  Close".into(),
                });
            }
            _ => {}
        }
        EventOutcome::Continue
    }

    async fn perform_ui_action(&mut self, action: UiAction) {
        match action {
            UiAction::Focus(Focus::GitHub) => self.focus_github(),
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
                self.maybe_queue_history();
            }
            UiAction::SelectBranch(index) => {
                self.focus = Focus::Branches;
                self.selected_branch = index;
            }
            UiAction::SelectStash(index) => {
                self.focus = Focus::Stashes;
                self.selected_stash = index;
            }
            UiAction::SelectWorktree(index) => {
                self.focus = Focus::Worktrees;
                self.selected_worktree = index;
            }
            UiAction::SelectPullRequest(index) | UiAction::SelectIssue(index) => {
                self.focus = Focus::GitHub;
                self.selected_github = index;
            }
            UiAction::Refresh => self.queue_refresh(true),
            UiAction::Fetch => self.run_remote("Fetching", RemoteAction::Fetch),
            UiAction::Pull => self.run_remote("Pulling", RemoteAction::Pull),
            UiAction::Push => self.run_contextual_push(),
            UiAction::Commit => self.commit(CommitOptions::default()).await,
            UiAction::StageAll => self.run_stage_all().await,
            UiAction::UnstageAll => self.run_unstage_all().await,
            UiAction::ToggleHelp => self.open_help(),
            UiAction::CloseOverlay => self.overlay = None,
            UiAction::PublishGitHub => self.begin_publish_github(),
            UiAction::ConfirmGitHubVisibility(visibility) => {
                if let Some(Overlay::GitHubVisibility { name, .. }) = self.overlay.take() {
                    self.request_publish_github(name, visibility).await;
                }
            }
        }
    }

    async fn activate(&mut self) {
        match self.focus {
            Focus::Changes | Focus::Staged => self.open_change_preview().await,
            Focus::Graph => self.open_commit_preview().await,
            Focus::Branches => self.checkout_selected().await,
            Focus::Stashes => self.open_stash_preview().await,
            Focus::Worktrees => self.status_line = "Use X to remove a linked worktree".into(),
            Focus::GitHub if self.active().github_state == GitHubConnectionState::NoRemote => {
                self.begin_publish_github()
            }
            Focus::GitHub => self.open_github_preview().await,
            Focus::Preview => {}
            _ => {}
        }
    }

    fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help {
            scroll: 0,
            max_scroll: 0,
        });
    }

    fn repeat_search(&mut self) {
        let Some(query) = self.last_search.clone() else {
            self.status_line = "Press / to search the focused view".into();
            return;
        };
        self.search_focused(query, true);
    }

    fn search_focused(&mut self, query: String, after_current: bool) {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            self.status_line = "Search text cannot be empty".into();
            return;
        }
        self.last_search = Some(query.clone());
        if self.focus == Focus::Preview {
            let Some(preview) = &mut self.preview else {
                return;
            };
            let start = if after_current {
                usize::from(preview.scroll).saturating_add(1)
            } else {
                usize::from(preview.scroll)
            };
            let lines = preview.body.lines().collect::<Vec<_>>();
            if let Some(index) = (0..lines.len()).find_map(|offset| {
                let index = (start + offset) % lines.len().max(1);
                lines[index]
                    .to_lowercase()
                    .contains(&query)
                    .then_some(index)
            }) {
                preview.scroll = index.min(u16::MAX as usize) as u16;
                self.status_line = format!("Found {query}");
            } else {
                self.status_line = format!("No match for {query}");
            }
            return;
        }
        let len = self.current_len();
        if len == 0 {
            self.status_line = format!("No match for {query}");
            return;
        }
        let start = if after_current {
            (self.current_selection() + 1) % len
        } else {
            self.current_selection().min(len - 1)
        };
        if let Some(index) = (0..len).find_map(|offset| {
            let index = (start + offset) % len;
            self.search_text(index)
                .to_lowercase()
                .contains(&query)
                .then_some(index)
        }) {
            self.set_selection(index);
            self.status_line = format!("Found {query}");
        } else {
            self.status_line = format!("No match for {query}");
        }
    }

    fn search_text(&self, index: usize) -> String {
        match self.focus {
            Focus::Changes if !self.active().status.conflicts.is_empty() => self
                .active()
                .status
                .conflicts
                .get(index)
                .map(|change| change.path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Focus::Changes => self
                .active()
                .status
                .unstaged
                .get(index)
                .map(|change| change.path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Focus::Staged => self
                .active()
                .status
                .staged
                .get(index)
                .map(|change| change.path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Focus::Graph => self
                .active()
                .history
                .get(index)
                .map(|commit| {
                    format!(
                        "{} {} {} {}",
                        commit.oid,
                        commit.subject,
                        commit.author,
                        commit.decorations.join(" ")
                    )
                })
                .unwrap_or_default(),
            Focus::Branches => self
                .active()
                .branches
                .get(index)
                .map(|branch| branch.name.clone())
                .unwrap_or_default(),
            Focus::Stashes => self
                .active()
                .stashes
                .get(index)
                .map(|stash| format!("{} {}", stash.reference, stash.subject))
                .unwrap_or_default(),
            Focus::Worktrees => self
                .active()
                .worktrees
                .get(index)
                .map(|worktree| {
                    format!(
                        "{} {}",
                        worktree.path.display(),
                        worktree.branch.as_deref().unwrap_or_default()
                    )
                })
                .unwrap_or_default(),
            Focus::GitHub if self.github_show_issues => self
                .active()
                .issues
                .get(index)
                .map(|issue| format!("{} {} {}", issue.number, issue.title, issue.author))
                .unwrap_or_default(),
            Focus::GitHub => self
                .active()
                .pull_requests
                .get(index)
                .map(|pr| format!("{} {} {} {}", pr.number, pr.title, pr.author, pr.head))
                .unwrap_or_default(),
            Focus::Commit | Focus::Preview => String::new(),
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

    fn conflicts_focused(&self) -> bool {
        self.focus == Focus::Changes && !self.active().status.conflicts.is_empty()
    }

    fn operation_focused(&self) -> bool {
        self.focus == Focus::Changes && self.active().status.operation.is_some()
    }

    async fn resolve_selected_conflict(&mut self, choice: ConflictChoice) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        let result = repo.resolve_conflict(&change.path, choice).await;
        self.finish_action(result, &format!("Resolved {}", change.path.display()))
            .await;
    }

    async fn continue_operation(&mut self) {
        let Some(operation) = self.active().status.operation else {
            self.status_line = "No merge, rebase, cherry-pick, or revert is in progress".into();
            return;
        };
        if !self.active().status.conflicts.is_empty() {
            self.status_line = "Resolve every conflict before continuing".into();
            return;
        }
        let repo = self.active().repo.clone();
        let result = repo.continue_operation(operation).await;
        self.finish_action(result, "Continued Git operation").await;
    }

    async fn abort_operation(&mut self) {
        let Some(operation) = self.active().status.operation else {
            self.status_line = "No merge, rebase, cherry-pick, or revert is in progress".into();
            return;
        };
        let repo = self.active().repo.clone();
        let result = repo.abort_operation(operation).await;
        self.finish_action(result, "Aborted Git operation").await;
    }

    async fn skip_operation(&mut self) {
        let Some(operation) = self.active().status.operation else {
            return;
        };
        let repo = self.active().repo.clone();
        let result = repo.skip_operation(operation).await;
        self.finish_action(result, "Skipped current Git operation step")
            .await;
    }

    fn recall_commit_message(&mut self, amount: isize) {
        if self.active().history.is_empty() {
            return;
        }
        if self.commit_history_index.is_none() {
            self.commit_history_draft = self.commit_message.clone();
        }
        let next = match (self.commit_history_index, amount.is_negative()) {
            (None, true) => 0,
            (None, false) => return,
            (Some(0), false) => {
                self.commit_history_index = None;
                self.commit_message = self.commit_history_draft.clone();
                return;
            }
            (Some(index), true) => (index + 1).min(self.active().history.len() - 1),
            (Some(index), false) => index - 1,
        };
        self.commit_history_index = Some(next);
        self.commit_message = self.active().history[next].subject.clone();
    }

    fn open_diagnostics(&mut self) {
        let body = if self.output.is_empty() {
            "No Git errors have been recorded in this session.".into()
        } else {
            self.output.join("\n\n")
        };
        self.preview = Some(Preview {
            title: "Git diagnostics".into(),
            body,
            scroll: 0,
            change: None,
            hunks: Vec::new(),
            selected_hunk: 0,
        });
        self.focus = Focus::Preview;
    }

    fn open_remote_prompt(&mut self, push: bool) {
        let remote = self
            .active()
            .remotes
            .iter()
            .find(|remote| remote.name == "origin")
            .or_else(|| self.active().remotes.first())
            .map(|remote| remote.name.as_str())
            .unwrap_or("origin");
        let branch = self
            .active()
            .status
            .branch
            .head
            .as_deref()
            .unwrap_or("main");
        self.overlay = Some(Overlay::Prompt {
            title: if push { "Push to" } else { "Pull from" }.into(),
            label: format!("Remote and branch (default: {remote} {branch})"),
            value: String::new(),
            replace_on_type: false,
            action: if push {
                PromptAction::PushTarget {
                    remote: remote.into(),
                    branch: branch.into(),
                }
            } else {
                PromptAction::PullTarget {
                    remote: remote.into(),
                    branch: branch.into(),
                }
            },
        });
    }

    async fn request_force_push(&mut self) {
        let Some(upstream) = self.active().status.branch.upstream.clone() else {
            self.status_line = "Publish the branch before force-pushing".into();
            return;
        };
        let Some((remote, branch)) = upstream.split_once('/') else {
            self.status_line = "Could not determine the upstream target".into();
            return;
        };
        self.confirm_or_execute(
            format!("Force-push with lease to {upstream}? [y/N]"),
            ConfirmAction::ForcePush {
                remote: remote.into(),
                branch: branch.into(),
            },
        )
        .await;
    }

    async fn request_undo_last_commit(&mut self) {
        if self.active().history.is_empty() {
            self.status_line = "There is no commit to undo".into();
            return;
        }
        self.confirm_or_execute(
            "Undo the last commit and keep its changes in the working tree? [y/N]".into(),
            ConfirmAction::UndoLastCommit,
        )
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

    async fn commit(&mut self, options: CommitOptions) {
        let repo = self.active().repo.clone();
        let result = repo.commit(&self.commit_message, options).await;
        if result.is_ok() {
            self.commit_message.clear();
            self.focus = Focus::Changes;
        }
        let success = match (options.amend, options.signoff) {
            (true, true) => "Amended commit with sign-off",
            (true, false) => "Amended commit",
            (false, true) => "Committed changes with sign-off",
            (false, false) => "Committed changes",
        };
        self.finish_action(result, success).await;
    }

    fn run_remote(&mut self, label: &'static str, action: RemoteAction) {
        if self.background_task.is_some() {
            self.status_line = "Another Git operation is still running".into();
            return;
        }
        self.status_line = format!("{label}…");
        self.busy = true;
        let repo_index = self.active_repo;
        let repo = self.active().repo.clone();
        self.background_task = Some(tokio::spawn(async move {
            let result = match action {
                RemoteAction::Fetch => repo.fetch().await,
                RemoteAction::Pull => repo.pull().await,
                RemoteAction::PullRebase => repo.pull_rebase().await,
                RemoteAction::PullFrom { remote, branch } => {
                    repo.pull_from(&remote, &branch, false).await
                }
                RemoteAction::Push => repo.push().await,
                RemoteAction::PushTo {
                    remote,
                    branch,
                    force_with_lease,
                } => repo.push_to(&remote, &branch, force_with_lease).await,
                RemoteAction::Publish { remote, branch } => repo.publish(&remote, &branch).await,
                RemoteAction::Sync => repo.sync().await,
            };
            BackgroundResult::Remote {
                repo_index,
                label,
                result,
            }
        }));
    }

    fn run_contextual_push(&mut self) {
        let status = &self.active().status.branch;
        if status.upstream.is_none() {
            let Some(branch) = status.head.clone() else {
                self.status_line = "Create or switch to a branch before publishing".into();
                return;
            };
            let remote = self
                .active()
                .remotes
                .iter()
                .find(|remote| remote.name == "origin")
                .or_else(|| self.active().remotes.first())
                .map(|remote| remote.name.clone());
            let Some(remote) = remote else {
                if self.active().github_state == GitHubConnectionState::NoRemote {
                    self.begin_publish_github();
                    return;
                }
                self.status_line = "Add a remote before publishing this branch".into();
                return;
            };
            self.run_remote("Publishing", RemoteAction::Publish { remote, branch });
        } else if status.ahead > 0 && status.behind > 0 {
            self.run_remote("Syncing", RemoteAction::Sync);
        } else {
            self.run_remote("Pushing", RemoteAction::Push);
        }
    }

    pub fn push_control_label(&self) -> &'static str {
        let status = &self.active().status.branch;
        if status.upstream.is_none() {
            "Publish"
        } else if status.ahead > 0 && status.behind > 0 {
            "Sync"
        } else {
            "Push"
        }
    }

    async fn run_stash(&mut self) {
        let repo = self.active().repo.clone();
        let result = repo.stash().await;
        self.finish_action(result, "Created stash").await;
    }

    async fn open_stash_preview(&mut self) {
        let Some(stash) = self.active().stashes.get(self.selected_stash).cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        match repo.show_stash(&stash.reference).await {
            Ok(body) => {
                self.preview = Some(Preview {
                    title: format!("{} {}", stash.reference, stash.subject),
                    hunks: split_diff_hunks(&body),
                    body,
                    scroll: 0,
                    change: None,
                    selected_hunk: 0,
                });
                self.focus = Focus::Preview;
            }
            Err(error) => self.report_error(error),
        }
    }

    async fn apply_selected_stash(&mut self, pop: bool) {
        let Some(stash) = self.active().stashes.get(self.selected_stash).cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        let result = if pop {
            repo.pop_stash(&stash.reference).await
        } else {
            repo.apply_stash(&stash.reference).await
        };
        self.finish_action(
            result,
            &format!(
                "{} {}",
                if pop { "Popped" } else { "Applied" },
                stash.reference
            ),
        )
        .await;
    }

    async fn request_drop_stash(&mut self) {
        let Some(stash) = self.active().stashes.get(self.selected_stash) else {
            return;
        };
        let prompt = format!(
            "Permanently drop {} ({})? [y/N]",
            stash.reference, stash.subject
        );
        let action = ConfirmAction::DropStash {
            reference: stash.reference.clone(),
        };
        self.confirm_or_execute(prompt, action).await;
    }

    async fn request_remove_worktree(&mut self) {
        let Some(worktree) = self.active().worktrees.get(self.selected_worktree) else {
            return;
        };
        if worktree.path == self.active().repo.root() {
            self.status_line = "The active worktree cannot be removed".into();
            return;
        }
        let prompt = format!(
            "Remove linked worktree {}?\nGit will refuse if it has changes. [y/N]",
            worktree.path.display()
        );
        let action = ConfirmAction::RemoveWorktree {
            path: worktree.path.clone(),
        };
        self.confirm_or_execute(prompt, action).await;
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

    async fn request_delete_branch(&mut self) {
        let Some(branch) = self.active().branches.get(self.selected_branch) else {
            return;
        };
        if branch.current || branch.remote {
            self.status_line = if branch.current {
                "The current branch cannot be deleted".into()
            } else {
                "Delete remote branches with an explicit remote push".into()
            };
            return;
        }
        let prompt = format!(
            "Delete local branch {}?\nGit will refuse if it is not merged. [y/N]",
            branch.name
        );
        let action = ConfirmAction::DeleteBranch {
            name: branch.name.clone(),
        };
        self.confirm_or_execute(prompt, action).await;
    }

    async fn merge_selected(&mut self) {
        let Some(branch) = self.active().branches.get(self.selected_branch).cloned() else {
            return;
        };
        if branch.current {
            self.status_line = "Select another branch to merge into the current branch".into();
            return;
        }
        let repo = self.active().repo.clone();
        let result = repo.merge(&branch.name).await;
        self.finish_action(result, &format!("Merged {}", branch.name))
            .await;
    }

    async fn request_rebase(&mut self) {
        let Some(branch) = self.active().branches.get(self.selected_branch) else {
            return;
        };
        if branch.current {
            self.status_line = "Select the branch to rebase the current branch onto".into();
            return;
        }
        let prompt = format!(
            "Rebase the current branch onto {}?\nThis rewrites local commits. [y/N]",
            branch.name
        );
        let action = ConfirmAction::Rebase {
            branch: branch.name.clone(),
        };
        self.confirm_or_execute(prompt, action).await;
    }

    async fn cherry_pick_selected(&mut self) {
        let Some(commit) = self.active().history.get(self.selected_commit).cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        let result = repo.cherry_pick(&commit.oid).await;
        self.finish_action(result, &format!("Cherry-picked {}", short_oid(&commit.oid)))
            .await;
    }

    async fn revert_selected(&mut self) {
        let Some(commit) = self.active().history.get(self.selected_commit).cloned() else {
            return;
        };
        let repo = self.active().repo.clone();
        let result = repo.revert(&commit.oid).await;
        self.finish_action(result, &format!("Reverted {}", short_oid(&commit.oid)))
            .await;
    }

    async fn execute_prompt(&mut self, action: PromptAction, value: String) {
        let value = value.trim();
        let remote_prompt = matches!(
            &action,
            PromptAction::PushTarget { .. } | PromptAction::PullTarget { .. }
        );
        if value.is_empty() && !remote_prompt {
            self.status_line = "A name is required".into();
            return;
        }
        let repo = self.active().repo.clone();
        match action {
            PromptAction::CreateBranch => {
                let result = repo.create_branch(value).await;
                self.finish_action(result, &format!("Created branch {value}"))
                    .await;
            }
            PromptAction::CreateTag { oid } => {
                let result = repo.create_tag(value, &oid).await;
                self.finish_action(result, &format!("Created tag {value}"))
                    .await;
            }
            PromptAction::AddWorktree { branch } => {
                let path = PathBuf::from(value);
                let result = repo.add_worktree(&path, &branch).await;
                self.finish_action(result, &format!("Added worktree {}", path.display()))
                    .await;
            }
            PromptAction::PushTarget { remote, branch } => {
                let Ok((remote, branch)) = parse_remote_target(value, remote, branch) else {
                    self.status_line = "Enter a remote and branch separated by a space".into();
                    return;
                };
                self.run_remote(
                    "Pushing to target",
                    RemoteAction::PushTo {
                        remote,
                        branch,
                        force_with_lease: false,
                    },
                );
            }
            PromptAction::PullTarget { remote, branch } => {
                let Ok((remote, branch)) = parse_remote_target(value, remote, branch) else {
                    self.status_line = "Enter a remote and branch separated by a space".into();
                    return;
                };
                self.run_remote(
                    "Pulling from target",
                    RemoteAction::PullFrom { remote, branch },
                );
            }
            PromptAction::PublishGitHubName => {
                if !valid_github_repository_name(value) {
                    self.status_line =
                        "Use letters, numbers, '.', '-' or '_' for the repository name".into();
                    return;
                }
                self.overlay = Some(Overlay::GitHubVisibility {
                    name: value.into(),
                    selected: GitHubVisibility::Private,
                });
            }
        }
    }

    async fn request_discard(&mut self) {
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
        let prompt = format!(
            "Discard all working-tree changes in {}?\nThis cannot be undone. [y/N]",
            change.path.display()
        );
        self.confirm_or_execute(prompt, action).await;
    }

    async fn confirm_or_execute(&mut self, prompt: String, action: ConfirmAction) {
        if self.settings.confirm_destructive {
            self.overlay = Some(Overlay::Confirm { prompt, action });
        } else {
            self.execute_confirmed(action).await;
        }
    }

    async fn execute_confirmed(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Discard { path, untracked } => {
                let repo = self.active().repo.clone();
                let result = repo.discard(&path, untracked).await;
                self.finish_action(result, "Discarded change").await;
            }
            ConfirmAction::DeleteBranch { name } => {
                let repo = self.active().repo.clone();
                let result = repo.delete_branch(&name).await;
                self.finish_action(result, &format!("Deleted branch {name}"))
                    .await;
            }
            ConfirmAction::Rebase { branch } => {
                let repo = self.active().repo.clone();
                let result = repo.rebase(&branch).await;
                self.finish_action(result, &format!("Rebased onto {branch}"))
                    .await;
            }
            ConfirmAction::DropStash { reference } => {
                let repo = self.active().repo.clone();
                let result = repo.drop_stash(&reference).await;
                self.finish_action(result, &format!("Dropped {reference}"))
                    .await;
            }
            ConfirmAction::RemoveWorktree { path } => {
                let repo = self.active().repo.clone();
                let result = repo.remove_worktree(&path).await;
                self.finish_action(result, &format!("Removed worktree {}", path.display()))
                    .await;
            }
            ConfirmAction::UndoLastCommit => {
                let repo = self.active().repo.clone();
                let result = repo.undo_last_commit().await;
                self.finish_action(result, "Undid last commit and kept its changes")
                    .await;
            }
            ConfirmAction::ForcePush { remote, branch } => {
                self.run_remote(
                    "Force-pushing with lease",
                    RemoteAction::PushTo {
                        remote,
                        branch,
                        force_with_lease: true,
                    },
                );
            }
            ConfirmAction::PublishGitHub {
                name,
                visibility,
                remote,
                push,
            } => {
                self.queue_publish_github(name, visibility, remote, push);
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

    fn begin_publish_github(&mut self) {
        if self.active().github_state != GitHubConnectionState::NoRemote {
            self.load_github();
            return;
        }
        self.overlay = Some(Overlay::Prompt {
            title: "Publish to GitHub".into(),
            label: "Repository name".into(),
            value: self.active().repo.name(),
            action: PromptAction::PublishGitHubName,
            replace_on_type: true,
        });
    }

    async fn request_publish_github(&mut self, name: String, visibility: GitHubVisibility) {
        if self.active().github_state != GitHubConnectionState::NoRemote {
            self.status_line = "GitHub connection changed; refresh and try again".into();
            return;
        }
        let remote = github_remote_name(&self.active().remotes);
        let push = !self.active().history.is_empty() && self.active().status.branch.head.is_some();
        let visibility_label = match visibility {
            GitHubVisibility::Private => "private",
            GitHubVisibility::Public => "public",
        };
        let dirty = !self.active().status.staged.is_empty()
            || !self.active().status.unstaged.is_empty()
            || !self.active().status.conflicts.is_empty();
        let mut prompt = format!(
            "Create {name} as a {visibility_label} GitHub repository using remote '{remote}'?"
        );
        if push {
            prompt.push_str("\nThe current branch and committed history will be pushed.");
        } else {
            prompt.push_str("\nThe repository will be connected without pushing.");
        }
        if dirty {
            prompt.push_str("\nUncommitted changes will remain local.");
        }
        prompt.push_str("\n\n[y/N]");
        // Publishing creates an external repository, so it always requires an
        // explicit confirmation even when local destructive confirmations are disabled.
        self.overlay = Some(Overlay::Confirm {
            prompt,
            action: ConfirmAction::PublishGitHub {
                name,
                visibility,
                remote,
                push,
            },
        });
    }

    fn queue_publish_github(
        &mut self,
        name: String,
        visibility: GitHubVisibility,
        remote: String,
        push: bool,
    ) {
        if self.background_task.is_some() {
            self.status_line = "Another Git operation is still running".into();
            return;
        }
        if self.active().github_state != GitHubConnectionState::NoRemote {
            self.status_line = "A GitHub remote is already configured".into();
            return;
        }
        self.status_line = "Publishing to GitHub…".into();
        self.busy = true;
        let repo_index = self.active_repo;
        let github = GitHub::new(self.active().repo.root());
        self.background_task = Some(tokio::spawn(async move {
            BackgroundResult::PublishGitHub {
                repo_index,
                result: github
                    .publish_repository(&name, visibility, &remote, push)
                    .await,
            }
        }));
    }

    fn load_github(&mut self) {
        if self.active().github_state != GitHubConnectionState::Ready {
            self.status_line = match self.active().github_state {
                GitHubConnectionState::CliMissing => "GitHub CLI is not installed",
                GitHubConnectionState::Unauthenticated => "GitHub CLI is not authenticated",
                GitHubConnectionState::NoRemote => "Publish or add a GitHub remote to continue",
                GitHubConnectionState::Ready => unreachable!(),
            }
            .into();
            return;
        }
        if self.active().github_loaded {
            return;
        }
        if self.background_task.is_some() {
            self.status_line = "Another Git operation is still running".into();
            return;
        }
        self.status_line = "Loading GitHub…".into();
        self.busy = true;
        let repo_index = self.active_repo;
        let github = GitHub::new(self.active().repo.root());
        self.background_task = Some(tokio::spawn(async move {
            let result = async {
                let (pull_requests, issues) =
                    tokio::try_join!(github.pull_requests(), github.issues())?;
                Ok((pull_requests, issues))
            }
            .await;
            BackgroundResult::GitHub { repo_index, result }
        }));
    }

    fn focus_github(&mut self) {
        self.focus = Focus::GitHub;
        if self.active().github_state == GitHubConnectionState::Ready {
            self.load_github();
        } else {
            // Recheck immediately when the panel is entered so a newly installed
            // or authenticated `gh` does not have to wait for the safety poll.
            self.queue_refresh(false);
        }
    }

    async fn open_github_preview(&mut self) {
        let github = GitHub::new(self.active().repo.root());
        let result = if self.github_show_issues {
            let Some(issue) = self.active().issues.get(self.selected_github).cloned() else {
                return;
            };
            github
                .issue_detail(issue.number)
                .await
                .map(|body| (format!("#{} {}", issue.number, issue.title), body))
        } else {
            let Some(pr) = self
                .active()
                .pull_requests
                .get(self.selected_github)
                .cloned()
            else {
                return;
            };
            github
                .pull_request_detail(pr.number)
                .await
                .map(|body| (format!("#{} {}", pr.number, pr.title), body))
        };
        match result {
            Ok((title, body)) => {
                self.preview = Some(Preview {
                    title,
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

    async fn open_github_in_browser(&mut self) {
        let github = GitHub::new(self.active().repo.root());
        let result = if self.github_show_issues {
            let Some(issue) = self.active().issues.get(self.selected_github) else {
                return;
            };
            github.open_issue(issue.number).await
        } else {
            let Some(pr) = self.active().pull_requests.get(self.selected_github) else {
                return;
            };
            github.open_pull_request(pr.number).await
        };
        match result {
            Ok(()) => self.status_line = "Opened in browser".into(),
            Err(error) => self.report_error(error),
        }
    }

    async fn checkout_pull_request(&mut self) {
        let Some(pr) = self
            .active()
            .pull_requests
            .get(self.selected_github)
            .cloned()
        else {
            return;
        };
        let github = GitHub::new(self.active().repo.root());
        let result = github.checkout_pull_request(pr.number).await;
        self.finish_action(result, &format!("Checked out PR #{}", pr.number))
            .await;
    }

    async fn open_pull_request_checks(&mut self) {
        let Some(pr) = self
            .active()
            .pull_requests
            .get(self.selected_github)
            .cloned()
        else {
            return;
        };
        let github = GitHub::new(self.active().repo.root());
        match github.pull_request_checks(pr.number).await {
            Ok(body) => {
                self.preview = Some(Preview {
                    title: format!("Checks for PR #{}", pr.number),
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

    pub async fn open_selected_in_difftool(&mut self) -> Result<()> {
        let change = self
            .editor_change()
            .cloned()
            .context("no changed file selected")?;
        self.active().repo.external_diff(&change).await
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

    pub async fn interactively_stage_selected(&mut self) -> Result<()> {
        let change = self
            .editor_change()
            .cloned()
            .context("no changed file selected")?;
        self.active().repo.interactive_stage(&change).await
    }

    async fn next_focus(&mut self, backwards: bool) {
        const ORDER: [Focus; 8] = [
            Focus::Commit,
            Focus::Changes,
            Focus::Staged,
            Focus::Graph,
            Focus::Branches,
            Focus::Stashes,
            Focus::Worktrees,
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
            self.focus_github();
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
            Focus::Stashes => self.selected_stash,
            Focus::Worktrees => self.selected_worktree,
            Focus::GitHub => self.selected_github,
            _ => self.selected_change,
        }
    }

    fn current_len(&self) -> usize {
        match self.focus {
            Focus::Changes if !self.active().status.conflicts.is_empty() => {
                self.active().status.conflicts.len()
            }
            Focus::Staged => self.active().status.staged.len(),
            Focus::Graph => self.active().history.len(),
            Focus::Branches => self.active().branches.len(),
            Focus::Stashes => self.active().stashes.len(),
            Focus::Worktrees => self.active().worktrees.len(),
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
            Focus::Stashes => self.selected_stash = clamped,
            Focus::Worktrees => self.selected_worktree = clamped,
            Focus::GitHub => self.selected_github = clamped,
            _ => self.selected_change = clamped,
        }
        self.maybe_queue_history();
    }

    fn maybe_queue_history(&mut self) {
        if self.focus != Focus::Graph || self.background_task.is_some() {
            return;
        }
        let view = self.active();
        if view.history_exhausted
            || view.history.is_empty()
            || self.selected_commit.saturating_add(5) < view.history.len()
        {
            return;
        }
        let page_size = self.settings.graph_page_size.max(1);
        let requested_limit = view.history_limit.saturating_add(page_size);
        let repo = view.repo.clone();
        let repo_index = self.active_repo;
        self.busy = true;
        self.status_line = "Loading more history…".into();
        self.background_task = Some(tokio::spawn(async move {
            BackgroundResult::History {
                repo_index,
                requested_limit,
                result: repo.history(requested_limit).await,
            }
        }));
    }

    fn clamp_selections(&mut self) {
        self.selected_change =
            self.selected_change
                .min(if self.active().status.conflicts.is_empty() {
                    self.active().status.unstaged.len().saturating_sub(1)
                } else {
                    self.active().status.conflicts.len().saturating_sub(1)
                });
        self.selected_staged = self
            .selected_staged
            .min(self.active().status.staged.len().saturating_sub(1));
        self.selected_commit = self
            .selected_commit
            .min(self.active().history.len().saturating_sub(1));
        self.selected_branch = self
            .selected_branch
            .min(self.active().branches.len().saturating_sub(1));
        self.selected_stash = self
            .selected_stash
            .min(self.active().stashes.len().saturating_sub(1));
        self.selected_worktree = self
            .selected_worktree
            .min(self.active().worktrees.len().saturating_sub(1));
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

async fn load_repo_snapshot(repo: GitRepo, history_limit: usize) -> Result<RepoSnapshot> {
    let (status, history, branches, remotes, stashes, worktrees) = tokio::try_join!(
        repo.status(),
        repo.history(history_limit),
        repo.branches(),
        repo.remotes(),
        repo.stashes(),
        repo.worktrees(),
    )?;
    let github_state = GitHub::new(repo.root()).connection_state(&remotes).await;
    Ok(RepoSnapshot {
        status,
        history,
        branches,
        remotes,
        stashes,
        worktrees,
        github_state,
    })
}

#[derive(Debug, Clone)]
enum RemoteAction {
    Fetch,
    Pull,
    PullRebase,
    PullFrom {
        remote: String,
        branch: String,
    },
    Push,
    PushTo {
        remote: String,
        branch: String,
        force_with_lease: bool,
    },
    Publish {
        remote: String,
        branch: String,
    },
    Sync,
}

fn commit_options_for_key(key: KeyEvent) -> Option<CommitOptions> {
    if key.code != KeyCode::Enter || !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    Some(CommitOptions {
        amend: key.modifiers.contains(KeyModifiers::SHIFT),
        signoff: key.modifiers.contains(KeyModifiers::ALT),
        ..CommitOptions::default()
    })
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

fn parse_remote_target(
    value: &str,
    default_remote: String,
    default_branch: String,
) -> Result<(String, String)> {
    if value.is_empty() {
        return Ok((default_remote, default_branch));
    }
    let fields = value.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2 {
        bail!("remote target requires a remote and branch");
    }
    Ok((fields[0].into(), fields[1].into()))
}

fn valid_github_repository_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 100
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn github_remote_name(remotes: &[Remote]) -> String {
    for candidate in std::iter::once("origin".to_owned())
        .chain(std::iter::once("github".to_owned()))
        .chain((2..).map(|suffix| format!("github-{suffix}")))
    {
        if remotes.iter().all(|remote| remote.name != candidate) {
            return candidate;
        }
    }
    unreachable!("the unbounded suffix sequence always contains a free remote name")
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
    use std::fs;

    use super::*;
    use clap::Parser;

    use crate::config::{Cli, Settings};

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

    #[test]
    fn maps_commit_modifier_combinations_without_a_menu() {
        let normal =
            commit_options_for_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)).unwrap();
        assert_eq!(normal, CommitOptions::default());

        let amend = commit_options_for_key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert!(amend.amend);
        assert!(!amend.signoff);

        let signoff = commit_options_for_key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ))
        .unwrap();
        assert!(!signoff.amend);
        assert!(signoff.signoff);

        assert!(
            commit_options_for_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).is_none()
        );
    }

    #[test]
    fn remote_target_accepts_defaults_or_an_explicit_pair() {
        assert_eq!(
            parse_remote_target("", "origin".into(), "main".into()).unwrap(),
            ("origin".into(), "main".into())
        );
        assert_eq!(
            parse_remote_target("upstream trunk", "origin".into(), "main".into()).unwrap(),
            ("upstream".into(), "trunk".into())
        );
        assert!(parse_remote_target("origin", "origin".into(), "main".into()).is_err());
    }

    #[test]
    fn validates_publish_names_and_avoids_overwriting_origin() {
        assert!(valid_github_repository_name("gitside.rs_2"));
        assert!(!valid_github_repository_name(""));
        assert!(!valid_github_repository_name("."));
        assert!(!valid_github_repository_name(".."));
        assert!(!valid_github_repository_name("owner/repository"));
        assert!(!valid_github_repository_name(&"x".repeat(101)));
        assert_eq!(github_remote_name(&[]), "origin");
        assert_eq!(
            github_remote_name(&[Remote {
                name: "origin".into(),
                fetch_url: "https://example.com/repository.git".into(),
                push_url: "https://example.com/repository.git".into(),
            }]),
            "github"
        );
        assert_eq!(
            github_remote_name(&[
                Remote {
                    name: "origin".into(),
                    fetch_url: String::new(),
                    push_url: String::new(),
                },
                Remote {
                    name: "github".into(),
                    fetch_url: String::new(),
                    push_url: String::new(),
                },
            ]),
            "github-2"
        );
    }

    #[tokio::test]
    async fn github_publish_flow_is_contextual_editable_and_always_confirmed() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let settings = Settings {
            confirm_destructive: false,
            ..Settings::default()
        };
        let mut app = App::new(cli, settings).await.unwrap();
        app.focus = Focus::GitHub;
        app.active_mut().github_state = GitHubConnectionState::NoRemote;
        app.active_mut().remotes.clear();

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
        let Some(Overlay::Prompt {
            value,
            replace_on_type,
            ..
        }) = &app.overlay
        else {
            panic!("publish should start with the repository-name prompt");
        };
        assert!(!value.is_empty());
        assert!(*replace_on_type);

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .await;
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Prompt {
                value,
                replace_on_type: false,
                ..
            }) if value == "x"
        ));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
        assert!(matches!(
            &app.overlay,
            Some(Overlay::GitHubVisibility {
                selected: GitHubVisibility::Private,
                ..
            })
        ));

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
            .await;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Confirm {
                action: ConfirmAction::PublishGitHub {
                    name,
                    visibility: GitHubVisibility::Public,
                    remote,
                    ..
                },
                ..
            }) if name == "x" && remote == "origin"
        ));
        assert!(app.background_task.is_none());
    }

    #[tokio::test]
    async fn publish_shortcut_creates_a_github_remote_when_none_exists() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.active_mut().github_state = GitHubConnectionState::NoRemote;
        app.active_mut().remotes.clear();
        app.active_mut().status.branch.head = Some("main".into());
        app.active_mut().status.branch.upstream = None;

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .await;

        assert!(matches!(
            &app.overlay,
            Some(Overlay::Prompt {
                action: PromptAction::PublishGitHubName,
                ..
            })
        ));
        assert!(app.background_task.is_none());
    }

    #[tokio::test]
    async fn commit_message_history_restores_the_users_draft() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.commit_message = "draft message".into();

        app.recall_commit_message(-1);
        assert_eq!(app.commit_message, app.active().history[0].subject);
        app.recall_commit_message(1);
        assert_eq!(app.commit_message, "draft message");
        assert!(app.commit_history_index.is_none());
    }

    #[tokio::test]
    async fn control_c_quits_from_normal_commit_and_overlay_contexts() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(app.handle_key(control_c).await, EventOutcome::Quit);
        app.focus = Focus::Commit;
        assert_eq!(app.handle_key(control_c).await, EventOutcome::Quit);
        app.overlay = Some(Overlay::Search {
            value: String::new(),
        });
        assert_eq!(app.handle_key(control_c).await, EventOutcome::Quit);
    }

    #[tokio::test]
    async fn remote_actions_are_queued_without_blocking_input() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();

        app.run_remote("Fetching", RemoteAction::Fetch);

        assert!(app.busy);
        assert!(app.background_task.is_some());
        assert_eq!(app.status_line, "Fetching…");
        app.background_task.take().unwrap().abort();
    }

    #[tokio::test]
    async fn manual_refresh_is_queued_and_applied_without_blocking_input() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();

        app.queue_refresh(true);

        assert!(app.busy);
        assert!(app.background_task.is_some());
        assert_eq!(app.status_line, "Refreshing…");
        while !app
            .background_task
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            tokio::task::yield_now().await;
        }
        assert!(app.poll_background().await);
        assert!(!app.busy);
        assert_eq!(app.status_line, "Repository refreshed");
    }

    #[tokio::test]
    async fn graph_boundary_queues_and_applies_another_history_page() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let settings = Settings {
            graph_page_size: 2,
            ..Settings::default()
        };
        let mut app = App::new(cli, settings).await.unwrap();
        assert_eq!(app.active().history.len(), 2);

        app.focus = Focus::Graph;
        app.set_selection(1);
        assert!(app.background_task.is_some());

        while !app
            .background_task
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            tokio::task::yield_now().await;
        }
        assert!(app.poll_background().await);

        assert!(app.active().history.len() >= 4);
        assert_eq!(app.active().history_limit, 4);
    }

    #[tokio::test]
    async fn disabling_confirmations_executes_destructive_actions_immediately() {
        let directory = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Gitside Test"],
            vec!["config", "user.email", "gitside@example.invalid"],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(directory.path())
                    .status()
                    .await
                    .unwrap()
                    .success()
            );
        }
        let file = directory.path().join("tracked.txt");
        fs::write(&file, "original\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "tracked.txt"])
                .current_dir(directory.path())
                .status()
                .await
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["commit", "-qm", "initial"])
                .current_dir(directory.path())
                .status()
                .await
                .unwrap()
                .success()
        );
        fs::write(&file, "modified\n").unwrap();

        let cli = Cli::try_parse_from(["gitside", directory.path().to_str().unwrap()]).unwrap();
        let settings = Settings {
            confirm_destructive: false,
            ..Settings::default()
        };
        let mut app = App::new(cli, settings).await.unwrap();
        app.request_discard().await;

        assert_eq!(fs::read_to_string(file).unwrap(), "original\n");
        assert!(app.overlay.is_none());
        assert!(app.active().status.unstaged.is_empty());
    }

    #[tokio::test]
    async fn slash_search_targets_only_the_focused_view_and_repeats() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.focus = Focus::Changes;
        app.active_mut().status.conflicts.clear();
        app.active_mut().status.unstaged = ["alpha.rs", "beta.rs", "alpha-test.rs"]
            .into_iter()
            .map(|path| Change {
                path: path.into(),
                original_path: None,
                kind: crate::model::ChangeKind::Modified,
                staged: false,
            })
            .collect();

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .await;
        assert!(matches!(app.overlay, Some(Overlay::Search { .. })));
        for character in "alpha".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
                .await;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
        assert_eq!(app.selected_change, 0);

        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.selected_change, 2);
        assert_eq!(app.last_search.as_deref(), Some("alpha"));
    }
}
