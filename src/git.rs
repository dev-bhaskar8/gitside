use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, anyhow, bail};
use tokio::{io::AsyncWriteExt, process::Command};

use crate::model::{
    Branch, Change, ChangeKind, Commit, GitOperation, Remote, RepoStatus, Stash, Worktree,
};

#[derive(Debug, Clone)]
pub struct GitRepo {
    root: PathBuf,
    git_dir: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum ConflictChoice {
    Current,
    Incoming,
    Both,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub all: bool,
    pub amend: bool,
    pub signoff: bool,
}

#[derive(Debug)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
}

impl GitRepo {
    pub async fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let output = Command::new("git")
            .args(["-C"])
            .arg(path)
            .args(["rev-parse", "--show-toplevel", "--absolute-git-dir"])
            .output()
            .await
            .with_context(|| "Git is required but was not found")?;
        if !output.status.success() {
            bail!(
                "{} is not inside a Git repository: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let resolved = String::from_utf8_lossy(&output.stdout);
        let mut lines = resolved.lines();
        let root = PathBuf::from(lines.next().unwrap_or_default());
        let git_dir = PathBuf::from(lines.next().unwrap_or_default());
        if root.as_os_str().is_empty() || git_dir.as_os_str().is_empty() {
            bail!("Git did not return repository paths for {}", path.display());
        }
        Ok(Self { root, git_dir })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .unwrap_or_else(|| self.root.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    async fn command<I, S>(&self, args: I, stdin: Option<&[u8]>) -> Result<CommandOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root).args(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        let mut child = command.spawn().context("failed to start Git")?;
        if let Some(input) = stdin {
            child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("Git stdin was unavailable"))?
                .write_all(input)
                .await?;
        }
        let output = child.wait_with_output().await?;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if !output.status.success() {
            bail!(
                "git exited with {}: {}",
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() {
                    "no diagnostic output"
                } else {
                    &stderr
                }
            );
        }
        Ok(CommandOutput {
            stdout: output.stdout,
        })
    }

    pub async fn status(&self) -> Result<RepoStatus> {
        let output = self
            .command(
                [
                    "status",
                    "--porcelain=v2",
                    "--branch",
                    "-z",
                    "--untracked-files=all",
                ],
                None,
            )
            .await?;
        let mut status = parse_status(&output.stdout)?;
        status.operation = self.operation_state().await;
        Ok(status)
    }

    async fn operation_state(&self) -> Option<GitOperation> {
        if tokio::fs::metadata(self.git_dir.join("rebase-merge"))
            .await
            .is_ok()
            || tokio::fs::metadata(self.git_dir.join("rebase-apply"))
                .await
                .is_ok()
        {
            return Some(GitOperation::Rebase);
        }
        for (file, operation) in [
            ("MERGE_HEAD", GitOperation::Merge),
            ("CHERRY_PICK_HEAD", GitOperation::CherryPick),
            ("REVERT_HEAD", GitOperation::Revert),
        ] {
            if tokio::fs::metadata(self.git_dir.join(file)).await.is_ok() {
                return Some(operation);
            }
        }
        None
    }

    pub async fn resolve_conflict(&self, path: &Path, choice: ConflictChoice) -> Result<()> {
        match choice {
            ConflictChoice::Current | ConflictChoice::Incoming => {
                let side = if matches!(choice, ConflictChoice::Current) {
                    "--ours"
                } else {
                    "--theirs"
                };
                self.command(
                    [
                        OsStr::new("checkout"),
                        OsStr::new(side),
                        OsStr::new("--"),
                        path.as_os_str(),
                    ],
                    None,
                )
                .await?;
            }
            ConflictChoice::Both => {
                let absolute = self.root.join(path);
                let contents = tokio::fs::read_to_string(&absolute)
                    .await
                    .with_context(|| format!("failed to read conflict {}", path.display()))?;
                let resolved = resolve_both_sides(&contents)?;
                tokio::fs::write(&absolute, resolved)
                    .await
                    .with_context(|| format!("failed to resolve conflict {}", path.display()))?;
            }
        }
        self.stage(path).await
    }

    pub async fn continue_operation(&self, operation: GitOperation) -> Result<()> {
        let args: &[&str] = match operation {
            GitOperation::Merge => &["commit", "--no-edit"],
            GitOperation::Rebase => &["-c", "core.editor=true", "rebase", "--continue"],
            GitOperation::CherryPick => &["cherry-pick", "--continue"],
            GitOperation::Revert => &["revert", "--continue"],
        };
        self.command(args, None).await?;
        Ok(())
    }

    pub async fn abort_operation(&self, operation: GitOperation) -> Result<()> {
        let args: &[&str] = match operation {
            GitOperation::Merge => &["merge", "--abort"],
            GitOperation::Rebase => &["rebase", "--abort"],
            GitOperation::CherryPick => &["cherry-pick", "--abort"],
            GitOperation::Revert => &["revert", "--abort"],
        };
        self.command(args, None).await?;
        Ok(())
    }

    pub async fn skip_operation(&self, operation: GitOperation) -> Result<()> {
        let args: &[&str] = match operation {
            GitOperation::Rebase => &["rebase", "--skip"],
            GitOperation::CherryPick => &["cherry-pick", "--skip"],
            GitOperation::Revert => &["revert", "--skip"],
            GitOperation::Merge => bail!("merge operations cannot be skipped"),
        };
        self.command(args, None).await?;
        Ok(())
    }

    pub async fn history(&self, limit: usize) -> Result<Vec<Commit>> {
        let format = "%H%x1f%P%x1f%D%x1f%s%x1f%an%x1f%cr%x1e";
        let history = self.command(
            vec![
                OsString::from("log"),
                OsString::from("--all"),
                OsString::from("--topo-order"),
                OsString::from(format!("--max-count={limit}")),
                OsString::from(format!("--format={format}")),
            ],
            None,
        );
        let remote_history = self.command(
            vec![
                OsString::from("log"),
                OsString::from("--remotes"),
                OsString::from("--topo-order"),
                OsString::from(format!("--max-count={limit}")),
                OsString::from("--format=%H"),
            ],
            None,
        );
        let (output, remote_output) = tokio::join!(history, remote_history);
        match output {
            Ok(value) => {
                let pushed = remote_output
                    .ok()
                    .map(|value| {
                        String::from_utf8_lossy(&value.stdout)
                            .lines()
                            .map(str::to_owned)
                            .collect::<HashSet<_>>()
                    })
                    .unwrap_or_default();
                let mut commits = parse_history(&value.stdout);
                for commit in &mut commits {
                    commit.pushed = pushed.contains(&commit.oid);
                }
                Ok(commits)
            }
            Err(error) if error.to_string().contains("does not have any commits yet") => Ok(vec![]),
            Err(error) if error.to_string().contains("unknown revision") => Ok(vec![]),
            Err(error) => Err(error),
        }
    }

    pub async fn branches(&self) -> Result<Vec<Branch>> {
        let output = self
            .command(
                [
                    "for-each-ref",
                    "--format=%(refname)%00%(HEAD)%00%(objectname)%00%(upstream:short)%00",
                    "refs/heads",
                    "refs/remotes",
                ],
                None,
            )
            .await?;
        Ok(parse_branches(&output.stdout))
    }

    pub async fn remotes(&self) -> Result<Vec<Remote>> {
        let output = self.command(["remote", "-v"], None).await?;
        Ok(parse_remotes(&output.stdout))
    }

    pub async fn diff(&self, change: &Change) -> Result<String> {
        let mut args = vec!["diff", "--no-ext-diff", "--no-color"];
        if change.staged {
            args.push("--cached");
        }
        args.push("--");
        let mut owned: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        owned.push(change.path.as_os_str().to_owned());
        let output = self.command(owned, None).await?;
        if output.stdout.is_empty() && change.kind == ChangeKind::Untracked {
            let path = self.root.join(&change.path);
            let content = tokio::fs::read(path).await?;
            let mut result = String::new();
            for line in String::from_utf8_lossy(&content).lines() {
                result.push('+');
                result.push_str(line);
                result.push('\n');
            }
            return Ok(result);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub async fn show_commit(&self, oid: &str) -> Result<String> {
        let output = self
            .command(
                [
                    "show",
                    "--no-ext-diff",
                    "--no-color",
                    "--stat",
                    "--patch",
                    oid,
                ],
                None,
            )
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub async fn stage(&self, path: &Path) -> Result<()> {
        self.stage_paths(&[path.to_path_buf()]).await
    }

    pub async fn stage_paths(&self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec![OsString::from("add"), OsString::from("--")];
        args.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
        self.command(args, None).await?;
        Ok(())
    }

    pub async fn unstage(&self, path: &Path) -> Result<()> {
        let result = self
            .command(
                [
                    OsStr::new("restore"),
                    OsStr::new("--staged"),
                    OsStr::new("--"),
                    path.as_os_str(),
                ],
                None,
            )
            .await;
        if result.is_err() {
            let reset = self
                .command(
                    [
                        OsStr::new("reset"),
                        OsStr::new("-q"),
                        OsStr::new("HEAD"),
                        OsStr::new("--"),
                        path.as_os_str(),
                    ],
                    None,
                )
                .await;
            if reset.is_err() {
                self.command(
                    [
                        OsStr::new("rm"),
                        OsStr::new("--cached"),
                        OsStr::new("-q"),
                        OsStr::new("--"),
                        path.as_os_str(),
                    ],
                    None,
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn unstage_paths(&self, paths: &[PathBuf]) -> Result<()> {
        for path in paths {
            self.unstage(path).await?;
        }
        Ok(())
    }

    pub async fn discard(&self, path: &Path, untracked: bool) -> Result<()> {
        if untracked {
            let absolute = self.root.join(path);
            if absolute.is_dir() {
                tokio::fs::remove_dir_all(absolute).await?;
            } else {
                tokio::fs::remove_file(absolute).await?;
            }
        } else {
            self.command(
                [
                    OsStr::new("restore"),
                    OsStr::new("--worktree"),
                    OsStr::new("--"),
                    path.as_os_str(),
                ],
                None,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn commit(&self, message: &str, options: CommitOptions) -> Result<()> {
        if message.trim().is_empty() {
            bail!("commit message cannot be empty");
        }
        let mut args = vec!["commit", "--file=-"];
        if options.all {
            args.push("--all");
        }
        if options.amend {
            args.push("--amend");
        }
        if options.signoff {
            args.push("--signoff");
        }
        self.command(args, Some(message.as_bytes())).await?;
        Ok(())
    }

    pub async fn checkout(&self, branch: &str) -> Result<()> {
        self.command(["switch", branch], None).await?;
        Ok(())
    }

    pub async fn create_branch(&self, name: &str) -> Result<()> {
        self.command(["switch", "--create", name], None).await?;
        Ok(())
    }

    pub async fn delete_branch(&self, name: &str) -> Result<()> {
        self.command(["branch", "--delete", name], None).await?;
        Ok(())
    }

    pub async fn merge(&self, branch: &str) -> Result<()> {
        self.command(["merge", branch], None).await?;
        Ok(())
    }

    pub async fn rebase(&self, branch: &str) -> Result<()> {
        self.command(["rebase", branch], None).await?;
        Ok(())
    }

    pub async fn cherry_pick(&self, oid: &str) -> Result<()> {
        self.command(["cherry-pick", oid], None).await?;
        Ok(())
    }

    pub async fn revert(&self, oid: &str) -> Result<()> {
        self.command(["revert", "--no-edit", oid], None).await?;
        Ok(())
    }

    pub async fn create_tag(&self, name: &str, oid: &str) -> Result<()> {
        self.command(["tag", name, oid], None).await?;
        Ok(())
    }

    pub async fn fetch(&self) -> Result<()> {
        self.command(["fetch", "--all", "--prune"], None).await?;
        Ok(())
    }

    pub async fn pull(&self) -> Result<()> {
        self.command(["pull"], None).await?;
        Ok(())
    }

    pub async fn pull_rebase(&self) -> Result<()> {
        self.command(["pull", "--rebase"], None).await?;
        Ok(())
    }

    pub async fn pull_from(&self, remote: &str, branch: &str, rebase: bool) -> Result<()> {
        let mut args = vec!["pull"];
        if rebase {
            args.push("--rebase");
        }
        args.extend([remote, branch]);
        self.command(args, None).await?;
        Ok(())
    }

    pub async fn push(&self) -> Result<()> {
        self.command(["push"], None).await?;
        Ok(())
    }

    pub async fn push_to(&self, remote: &str, branch: &str, force_with_lease: bool) -> Result<()> {
        let mut args = vec!["push"];
        if force_with_lease {
            args.push("--force-with-lease");
        }
        args.extend([remote, branch]);
        self.command(args, None).await?;
        Ok(())
    }

    pub async fn undo_last_commit(&self) -> Result<()> {
        self.command(["reset", "--mixed", "HEAD~1"], None).await?;
        Ok(())
    }

    pub async fn external_diff(&self, change: &Change) -> Result<()> {
        let mut args = vec![OsString::from("difftool"), OsString::from("--no-prompt")];
        if change.staged {
            args.push(OsString::from("--cached"));
        }
        args.push(OsString::from("--"));
        args.push(change.path.as_os_str().to_owned());
        self.interactive_command(args).await
    }

    pub async fn interactive_stage(&self, change: &Change) -> Result<()> {
        if !change.staged && change.kind == ChangeKind::Untracked {
            self.command(
                [
                    OsStr::new("add"),
                    OsStr::new("--intent-to-add"),
                    OsStr::new("--"),
                    change.path.as_os_str(),
                ],
                None,
            )
            .await?;
        }
        let command = if change.staged { "reset" } else { "add" };
        self.interactive_command([
            OsString::from(command),
            OsString::from("--patch"),
            OsString::from("--"),
            change.path.as_os_str().to_owned(),
        ])
        .await
    }

    async fn interactive_command<I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to start interactive Git command")?;
        if !status.success() {
            bail!(
                "interactive Git command exited with {}",
                status.code().unwrap_or(-1)
            );
        }
        Ok(())
    }

    pub async fn publish(&self, remote: &str, branch: &str) -> Result<()> {
        self.command(["push", "--set-upstream", remote, branch], None)
            .await?;
        Ok(())
    }

    pub async fn sync(&self) -> Result<()> {
        self.pull().await?;
        self.push().await
    }

    pub async fn stash(&self) -> Result<()> {
        self.command(["stash", "push", "--include-untracked"], None)
            .await?;
        Ok(())
    }

    pub async fn stashes(&self) -> Result<Vec<Stash>> {
        let output = self
            .command(["stash", "list", "--format=%gd%x00%gs%x00"], None)
            .await?;
        let fields: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
        Ok(fields
            .chunks(2)
            .filter_map(|pair| {
                if pair.len() < 2 || pair[0].is_empty() {
                    return None;
                }
                Some(Stash {
                    reference: String::from_utf8_lossy(pair[0])
                        .trim_start_matches('\n')
                        .to_owned(),
                    subject: String::from_utf8_lossy(pair[1]).into_owned(),
                })
            })
            .collect())
    }

    pub async fn show_stash(&self, reference: &str) -> Result<String> {
        let output = self
            .command(
                [
                    "stash",
                    "show",
                    "--patch",
                    "--stat",
                    "--no-color",
                    reference,
                ],
                None,
            )
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub async fn apply_stash(&self, reference: &str) -> Result<()> {
        self.command(["stash", "apply", reference], None).await?;
        Ok(())
    }

    pub async fn pop_stash(&self, reference: &str) -> Result<()> {
        self.command(["stash", "pop", reference], None).await?;
        Ok(())
    }

    pub async fn drop_stash(&self, reference: &str) -> Result<()> {
        self.command(["stash", "drop", reference], None).await?;
        Ok(())
    }

    pub async fn apply_cached_patch(&self, patch: &str, reverse: bool) -> Result<()> {
        let mut args = vec!["apply", "--cached", "--recount", "--whitespace=nowarn"];
        if reverse {
            args.push("--reverse");
        }
        self.command(args, Some(patch.as_bytes())).await?;
        Ok(())
    }

    pub async fn worktrees(&self) -> Result<Vec<Worktree>> {
        let output = self
            .command(["worktree", "list", "--porcelain", "-z"], None)
            .await?;
        Ok(parse_worktrees(&output.stdout))
    }

    pub async fn add_worktree(&self, path: &Path, branch: &str) -> Result<()> {
        self.command(
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                path.as_os_str(),
                OsStr::new(branch),
            ],
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn remove_worktree(&self, path: &Path) -> Result<()> {
        self.command(
            [
                OsStr::new("worktree"),
                OsStr::new("remove"),
                path.as_os_str(),
            ],
            None,
        )
        .await?;
        Ok(())
    }
}

fn parse_status(input: &[u8]) -> Result<RepoStatus> {
    let mut status = RepoStatus::default();
    let fields: Vec<&[u8]> = input.split(|byte| *byte == 0).collect();
    let mut index = 0;
    while index < fields.len() {
        let record = fields[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(record);
        if let Some(value) = text.strip_prefix("# branch.head ") {
            if value != "(detached)" && value != "(initial)" {
                status.branch.head = Some(value.to_owned());
            }
            continue;
        }
        if let Some(value) = text.strip_prefix("# branch.oid ") {
            if value != "(initial)" {
                status.branch.oid = Some(value.to_owned());
            }
            continue;
        }
        if let Some(value) = text.strip_prefix("# branch.upstream ") {
            status.branch.upstream = Some(value.to_owned());
            continue;
        }
        if let Some(value) = text.strip_prefix("# branch.ab ") {
            for component in value.split_whitespace() {
                if let Some(ahead) = component.strip_prefix('+') {
                    status.branch.ahead = ahead.parse().unwrap_or(0);
                } else if let Some(behind) = component.strip_prefix('-') {
                    status.branch.behind = behind.parse().unwrap_or(0);
                }
            }
            continue;
        }
        match record[0] {
            b'?' => status.unstaged.push(Change {
                path: bytes_to_path(record.get(2..).unwrap_or_default()),
                original_path: None,
                kind: ChangeKind::Untracked,
                staged: false,
            }),
            b'!' => {}
            b'1' | b'2' | b'u' => {
                let type_code = record[0];
                let field_count = match type_code {
                    b'1' => 9,
                    b'2' => 10,
                    b'u' => 11,
                    _ => unreachable!(),
                };
                let parts: Vec<&[u8]> = record.splitn(field_count, |byte| *byte == b' ').collect();
                if parts.len() != field_count {
                    continue;
                }
                let xy = parts[1];
                let path = parts[field_count - 1];
                let original_path = if type_code == b'2' && index < fields.len() {
                    let old = bytes_to_path(fields[index]);
                    index += 1;
                    Some(old)
                } else {
                    None
                };
                if type_code == b'u' {
                    let conflict = Change {
                        path: bytes_to_path(path),
                        original_path,
                        kind: ChangeKind::Conflicted,
                        staged: false,
                    };
                    status.conflicts.push(conflict.clone());
                    status.unstaged.push(conflict);
                    continue;
                }
                let x = *xy.first().unwrap_or(&b'.');
                let y = *xy.get(1).unwrap_or(&b'.');
                if x != b'.' {
                    status.staged.push(Change {
                        path: bytes_to_path(path),
                        original_path: original_path.clone(),
                        kind: kind_from_code(x),
                        staged: true,
                    });
                }
                if y != b'.' {
                    status.unstaged.push(Change {
                        path: bytes_to_path(path),
                        original_path,
                        kind: kind_from_code(y),
                        staged: false,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(status)
}

fn resolve_both_sides(contents: &str) -> Result<String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Section {
        Normal,
        Current,
        Base,
        Incoming,
    }

    let mut section = Section::Normal;
    let mut output = String::with_capacity(contents.len());
    let mut found_conflict = false;
    for line in contents.split_inclusive('\n') {
        if line.starts_with("<<<<<<< ") {
            if section != Section::Normal {
                bail!("nested conflict markers are not supported");
            }
            found_conflict = true;
            section = Section::Current;
        } else if line.starts_with("||||||| ") && section == Section::Current {
            section = Section::Base;
        } else if line.starts_with("=======") && matches!(section, Section::Current | Section::Base)
        {
            section = Section::Incoming;
        } else if line.starts_with(">>>>>>> ") && section == Section::Incoming {
            section = Section::Normal;
        } else if matches!(
            section,
            Section::Normal | Section::Current | Section::Incoming
        ) {
            output.push_str(line);
        }
    }
    if !found_conflict {
        bail!("the file contains no Git conflict markers");
    }
    if section != Section::Normal {
        bail!("the file contains incomplete Git conflict markers");
    }
    Ok(output)
}

fn kind_from_code(code: u8) -> ChangeKind {
    match code {
        b'A' => ChangeKind::Added,
        b'D' => ChangeKind::Deleted,
        b'R' => ChangeKind::Renamed,
        b'C' => ChangeKind::Copied,
        b'T' => ChangeKind::TypeChanged,
        b'U' => ChangeKind::Conflicted,
        _ => ChangeKind::Modified,
    }
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn parse_history(input: &[u8]) -> Vec<Commit> {
    input
        .split(|byte| *byte == 0x1e)
        .filter_map(|record| {
            let record = record.strip_prefix(b"\n").unwrap_or(record);
            if record.is_empty() {
                return None;
            }
            let fields: Vec<_> = record.split(|byte| *byte == 0x1f).collect();
            if fields.len() < 6 {
                return None;
            }
            Some(Commit {
                oid: String::from_utf8_lossy(fields[0]).into_owned(),
                parents: String::from_utf8_lossy(fields[1])
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                decorations: String::from_utf8_lossy(fields[2])
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect(),
                subject: String::from_utf8_lossy(fields[3]).into_owned(),
                author: String::from_utf8_lossy(fields[4]).into_owned(),
                relative_date: String::from_utf8_lossy(fields[5]).trim().to_owned(),
                pushed: false,
            })
        })
        .collect()
}

fn parse_branches(input: &[u8]) -> Vec<Branch> {
    input
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let fields: Vec<_> = line.split(|byte| *byte == 0).collect();
            if fields.len() < 4 {
                return None;
            }
            let refname = String::from_utf8_lossy(fields[0]);
            let remote = refname.starts_with("refs/remotes/");
            let name = refname
                .strip_prefix("refs/heads/")
                .or_else(|| refname.strip_prefix("refs/remotes/"))?
                .to_owned();
            if name.ends_with("/HEAD") {
                return None;
            }
            let upstream = String::from_utf8_lossy(fields[3]).to_string();
            Some(Branch {
                name,
                current: fields[1] == b"*",
                remote,
                oid: String::from_utf8_lossy(fields[2]).into_owned(),
                upstream: (!upstream.is_empty()).then_some(upstream),
            })
        })
        .collect()
}

fn parse_remotes(input: &[u8]) -> Vec<Remote> {
    use std::collections::BTreeMap;
    let mut values: BTreeMap<String, Remote> = BTreeMap::new();
    for line in String::from_utf8_lossy(input).lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(url), Some(direction)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let remote = values.entry(name.to_owned()).or_insert_with(|| Remote {
            name: name.to_owned(),
            fetch_url: String::new(),
            push_url: String::new(),
        });
        if direction == "(fetch)" {
            remote.fetch_url = url.to_owned();
        } else if direction == "(push)" {
            remote.push_url = url.to_owned();
        }
    }
    values.into_values().collect()
}

fn parse_worktrees(input: &[u8]) -> Vec<Worktree> {
    input
        .split(|byte| *byte == 0)
        .fold(
            (Vec::new(), None::<Worktree>),
            |(mut result, mut current), field| {
                if field.is_empty() {
                    if let Some(worktree) = current.take() {
                        result.push(worktree);
                    }
                    return (result, current);
                }
                let text = String::from_utf8_lossy(field);
                if let Some(path) = text.strip_prefix("worktree ") {
                    if let Some(worktree) = current.take() {
                        result.push(worktree);
                    }
                    current = Some(Worktree {
                        path: PathBuf::from(path),
                        head: String::new(),
                        branch: None,
                        locked: false,
                        prunable: false,
                    });
                } else if let Some(worktree) = current.as_mut() {
                    if let Some(head) = text.strip_prefix("HEAD ") {
                        worktree.head = head.to_owned();
                    } else if let Some(branch) = text.strip_prefix("branch refs/heads/") {
                        worktree.branch = Some(branch.to_owned());
                    } else if text.starts_with("locked") {
                        worktree.locked = true;
                    } else if text.starts_with("prunable") {
                        worktree.prunable = true;
                    }
                }
                (result, current)
            },
        )
        .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn parses_porcelain_v2_status() {
        let input = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 abc abc file.txt\0\
1 M. N... 100644 100644 100644 abc def staged.rs\0\
? new file.md\0";
        let status = parse_status(input).unwrap();
        assert_eq!(status.branch.head.as_deref(), Some("main"));
        assert_eq!(status.branch.ahead, 2);
        assert_eq!(status.branch.behind, 1);
        assert_eq!(status.staged[0].path, PathBuf::from("staged.rs"));
        assert_eq!(status.unstaged.len(), 2);
        assert_eq!(status.unstaged[1].path, PathBuf::from("new file.md"));
    }

    #[test]
    fn parses_history_records() {
        let input = b"abc\x1fdef 123\x1fHEAD -> main, tag: v1\x1fSubject\x1fAda\x1f2 hours ago\x1e";
        let commits = parse_history(input);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "Subject");
        assert_eq!(commits[0].decorations.len(), 2);
    }

    #[test]
    fn parses_conflicts_into_visible_changes() {
        let input = b"# branch.head main\0\
u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.txt\0";
        let status = parse_status(input).unwrap();
        assert_eq!(status.conflicts.len(), 1);
        assert_eq!(status.unstaged.len(), 1);
        assert_eq!(status.unstaged[0].kind, ChangeKind::Conflicted);
    }

    #[test]
    fn accepts_both_sides_of_standard_and_diff3_markers() {
        let conflict = "before\n<<<<<<< HEAD\ncurrent\n||||||| base\nbase\n=======\nincoming\n>>>>>>> feature\nafter\n";
        assert_eq!(
            resolve_both_sides(conflict).unwrap(),
            "before\ncurrent\nincoming\nafter\n"
        );
        assert!(resolve_both_sides("no conflict\n").is_err());
        assert!(resolve_both_sides("<<<<<<< HEAD\nbroken\n").is_err());
    }

    #[test]
    fn parses_worktree_porcelain() {
        let input = b"worktree /repo\0HEAD abc123\0branch refs/heads/main\0\0\
worktree /repo-feature\0HEAD def456\0branch refs/heads/feature\0locked reason\0\0";
        let worktrees = parse_worktrees(input);
        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert!(worktrees[1].locked);
    }

    async fn repository() -> (TempDir, GitRepo) {
        let directory = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Gitside Test"],
            vec!["config", "user.email", "gitside@example.invalid"],
        ] {
            let status = Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .status()
                .await
                .unwrap();
            assert!(status.success());
        }
        let repo = GitRepo::discover(directory.path()).await.unwrap();
        (directory, repo)
    }

    #[tokio::test]
    async fn stages_commits_and_reads_real_repository() {
        let (directory, repo) = repository().await;
        fs::write(directory.path().join("hello.txt"), "hello\n").unwrap();

        let status = repo.status().await.unwrap();
        assert_eq!(status.unstaged[0].kind, ChangeKind::Untracked);

        repo.stage(Path::new("hello.txt")).await.unwrap();
        let status = repo.status().await.unwrap();
        assert_eq!(status.staged.len(), 1);

        repo.commit("Initial test commit", CommitOptions::default())
            .await
            .unwrap();
        let history = repo.history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].subject, "Initial test commit");

        let original_branch = repo.status().await.unwrap().branch.head.unwrap();
        repo.create_branch("feature/test").await.unwrap();
        assert_eq!(
            repo.status().await.unwrap().branch.head.as_deref(),
            Some("feature/test")
        );
        repo.create_tag("test-v1", &history[0].oid).await.unwrap();
        repo.checkout(&original_branch).await.unwrap();
        repo.delete_branch("feature/test").await.unwrap();
        assert!(
            repo.branches()
                .await
                .unwrap()
                .iter()
                .all(|branch| branch.name != "feature/test")
        );

        fs::write(directory.path().join("hello.txt"), "hello\nworld\n").unwrap();
        let status = repo.status().await.unwrap();
        assert_eq!(status.unstaged[0].kind, ChangeKind::Modified);
        let diff = repo.diff(&status.unstaged[0]).await.unwrap();
        assert!(diff.contains("+world"));

        repo.stash().await.unwrap();
        let stashes = repo.stashes().await.unwrap();
        assert_eq!(stashes.len(), 1);
        assert!(
            repo.show_stash(&stashes[0].reference)
                .await
                .unwrap()
                .contains("+world")
        );
        repo.apply_stash(&stashes[0].reference).await.unwrap();
        assert_eq!(repo.status().await.unwrap().unstaged.len(), 1);
        repo.drop_stash(&stashes[0].reference).await.unwrap();
        assert!(repo.stashes().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn bulk_staging_only_adds_the_requested_snapshot_paths() {
        let (directory, repo) = repository().await;
        fs::write(directory.path().join("known.txt"), "known\n").unwrap();
        fs::write(directory.path().join("late.txt"), "late\n").unwrap();

        repo.stage_paths(&[PathBuf::from("known.txt")])
            .await
            .unwrap();
        let status = repo.status().await.unwrap();

        assert_eq!(status.staged.len(), 1);
        assert_eq!(status.staged[0].path, PathBuf::from("known.txt"));
        assert!(
            status
                .unstaged
                .iter()
                .any(|change| change.path == Path::new("late.txt"))
        );
    }

    #[tokio::test]
    async fn supports_amended_and_signed_off_commits() {
        let (directory, repo) = repository().await;
        fs::write(directory.path().join("message.txt"), "one\n").unwrap();
        repo.stage(Path::new("message.txt")).await.unwrap();
        repo.commit("Original subject", CommitOptions::default())
            .await
            .unwrap();

        fs::write(directory.path().join("message.txt"), "two\n").unwrap();
        repo.stage(Path::new("message.txt")).await.unwrap();
        repo.commit(
            "Amended subject",
            CommitOptions {
                amend: true,
                ..CommitOptions::default()
            },
        )
        .await
        .unwrap();
        let history = repo.history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].subject, "Amended subject");

        fs::write(directory.path().join("message.txt"), "three\n").unwrap();
        repo.stage(Path::new("message.txt")).await.unwrap();
        repo.commit(
            "Signed-off subject",
            CommitOptions {
                signoff: true,
                ..CommitOptions::default()
            },
        )
        .await
        .unwrap();
        let body = repo
            .command(["log", "-1", "--format=%B"], None)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body.stdout);
        assert!(body.contains("Signed-off-by: Gitside Test <gitside@example.invalid>"));
    }

    #[tokio::test]
    async fn detects_resolves_and_aborts_merge_conflicts() {
        let (directory, repo) = repository().await;
        let file = directory.path().join("conflict.txt");
        fs::write(&file, "base\n").unwrap();
        repo.stage(Path::new("conflict.txt")).await.unwrap();
        repo.commit("base", CommitOptions::default()).await.unwrap();
        let main = repo.status().await.unwrap().branch.head.unwrap();

        repo.create_branch("feature").await.unwrap();
        fs::write(&file, "incoming\n").unwrap();
        repo.commit(
            "feature",
            CommitOptions {
                all: true,
                ..CommitOptions::default()
            },
        )
        .await
        .unwrap();
        repo.checkout(&main).await.unwrap();
        fs::write(&file, "current\n").unwrap();
        repo.commit(
            "main",
            CommitOptions {
                all: true,
                ..CommitOptions::default()
            },
        )
        .await
        .unwrap();

        assert!(repo.merge("feature").await.is_err());
        let status = repo.status().await.unwrap();
        assert_eq!(status.operation, Some(GitOperation::Merge));
        assert_eq!(status.conflicts.len(), 1);

        repo.resolve_conflict(Path::new("conflict.txt"), ConflictChoice::Current)
            .await
            .unwrap();
        assert!(repo.status().await.unwrap().conflicts.is_empty());
        assert_eq!(fs::read_to_string(&file).unwrap(), "current\n");

        repo.abort_operation(GitOperation::Merge).await.unwrap();
        assert_eq!(repo.status().await.unwrap().operation, None);
        assert_eq!(fs::read_to_string(file).unwrap(), "current\n");
    }

    #[tokio::test]
    async fn publishes_an_upstream_and_syncs_diverged_branches() {
        let (directory, repo) = repository().await;
        let remote = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--bare", "-q"])
                .current_dir(remote.path())
                .status()
                .await
                .unwrap()
                .success()
        );
        repo.command(
            ["remote", "add", "origin", remote.path().to_str().unwrap()],
            None,
        )
        .await
        .unwrap();
        fs::write(directory.path().join("base.txt"), "base\n").unwrap();
        repo.stage(Path::new("base.txt")).await.unwrap();
        repo.commit("base", CommitOptions::default()).await.unwrap();
        let branch = repo.status().await.unwrap().branch.head.unwrap();
        repo.publish("origin", &branch).await.unwrap();
        let upstream = format!("origin/{branch}");
        assert_eq!(
            repo.status().await.unwrap().branch.upstream.as_deref(),
            Some(upstream.as_str())
        );

        let other = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args([
                    "clone",
                    "-q",
                    remote.path().to_str().unwrap(),
                    other.path().to_str().unwrap(),
                ])
                .status()
                .await
                .unwrap()
                .success()
        );
        for args in [
            ["config", "user.name", "Other Test"],
            ["config", "user.email", "other@example.invalid"],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(other.path())
                    .status()
                    .await
                    .unwrap()
                    .success()
            );
        }
        fs::write(other.path().join("remote.txt"), "remote\n").unwrap();
        for args in [
            vec!["add", "remote.txt"],
            vec!["commit", "-qm", "remote"],
            vec!["push", "-q"],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(other.path())
                    .status()
                    .await
                    .unwrap()
                    .success()
            );
        }
        fs::write(directory.path().join("local.txt"), "local\n").unwrap();
        repo.stage(Path::new("local.txt")).await.unwrap();
        repo.commit("local", CommitOptions::default())
            .await
            .unwrap();
        let history = repo.history(10).await.unwrap();
        assert!(
            history
                .iter()
                .find(|commit| commit.subject == "base")
                .is_some_and(|commit| commit.pushed)
        );
        assert!(
            history
                .iter()
                .find(|commit| commit.subject == "local")
                .is_some_and(|commit| !commit.pushed)
        );
        repo.command(["config", "pull.rebase", "false"], None)
            .await
            .unwrap();

        repo.sync().await.unwrap();
        let status = repo.status().await.unwrap();
        assert_eq!((status.branch.ahead, status.branch.behind), (0, 0));
        assert!(directory.path().join("remote.txt").exists());
    }

    #[tokio::test]
    async fn unstages_files_before_first_commit() {
        let (directory, repo) = repository().await;
        fs::write(directory.path().join("new.txt"), "new\n").unwrap();
        repo.stage(Path::new("new.txt")).await.unwrap();
        repo.unstage(Path::new("new.txt")).await.unwrap();
        let status = repo.status().await.unwrap();
        assert!(status.staged.is_empty());
        assert_eq!(status.unstaged[0].kind, ChangeKind::Untracked);
    }

    #[tokio::test]
    async fn undo_last_commit_keeps_its_changes() {
        let (directory, repo) = repository().await;
        fs::write(directory.path().join("first.txt"), "first\n").unwrap();
        repo.stage(Path::new("first.txt")).await.unwrap();
        repo.commit("first", CommitOptions::default())
            .await
            .unwrap();
        fs::write(directory.path().join("second.txt"), "second\n").unwrap();
        repo.stage(Path::new("second.txt")).await.unwrap();
        repo.commit("second", CommitOptions::default())
            .await
            .unwrap();

        repo.undo_last_commit().await.unwrap();

        assert_eq!(repo.history(10).await.unwrap()[0].subject, "first");
        assert!(directory.path().join("second.txt").exists());
        assert!(repo.status().await.unwrap().unstaged.iter().any(|change| {
            change.path == Path::new("second.txt") && change.kind == ChangeKind::Untracked
        }));
    }
}
