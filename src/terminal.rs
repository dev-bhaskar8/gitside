use std::{io, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, EventStream,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{sync::mpsc, time};

use crate::{
    app::{App, EventOutcome},
    ui,
};

struct TerminalGuard {
    mouse: bool,
}

impl TerminalGuard {
    fn enter(mouse: bool) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        if mouse {
            execute!(stdout, EnableMouseCapture)?;
        }
        Ok(Self { mouse })
    }

    fn leave(&self) -> Result<()> {
        let mut stdout = io::stdout();
        if self.mouse {
            execute!(stdout, DisableMouseCapture)?;
        }
        execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

pub async fn run(app: &mut App) -> Result<()> {
    let guard = TerminalGuard::enter(app.settings.mouse)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut events = EventStream::new();
    let (watch_tx, mut watch_events) = mpsc::channel(1);
    let watcher = start_repository_watcher(app, watch_tx);
    let watcher_active = watcher.is_some();
    let safety_refresh_ms = if watcher_active {
        app.settings.refresh_ms.max(30_000)
    } else {
        app.settings.refresh_ms.max(250)
    };
    let mut tick = time::interval(Duration::from_millis(safety_refresh_ms));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut background_tick = time::interval(Duration::from_millis(100));
    background_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let watch_debounce = Duration::from_millis(250);
    let mut watch_dirty = false;
    let mut last_watch_refresh = time::Instant::now() - watch_debounce;

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                let outcome = match event? {
                    Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                        app.handle_key(key).await
                    }
                    Event::Mouse(mouse) => app.handle_mouse(mouse).await,
                    Event::Resize(_, _) => EventOutcome::Continue,
                    Event::Paste(value) if app.focus == crate::app::Focus::Commit => {
                        app.commit_message.push_str(&value);
                        EventOutcome::Continue
                    }
                    _ => EventOutcome::Continue,
                };
                match outcome {
                    EventOutcome::Continue => {}
                    EventOutcome::Quit => break,
                    EventOutcome::OpenDifftool | EventOutcome::InteractiveStage => {
                        guard.leave()?;
                        let result = if matches!(outcome, EventOutcome::OpenDifftool) {
                            app.open_selected_in_difftool().await
                        } else {
                            app.interactively_stage_selected().await
                        };
                        if let Err(error) = result {
                            app.status_line = format!("{error:#}");
                        }
                        let _new_guard = TerminalGuard::enter(app.settings.mouse)?;
                        terminal.clear()?;
                        app.refresh().await;
                        std::mem::forget(_new_guard);
                    }
                    EventOutcome::OpenEditor => {
                        guard.leave()?;
                        if let Err(error) = app.open_selected_in_editor().await {
                            app.status_line = format!("{error:#}");
                        }
                        let _new_guard = TerminalGuard::enter(app.settings.mouse)?;
                        terminal.clear()?;
                        app.refresh().await;
                        std::mem::forget(_new_guard);
                    }
                }
            }
            _ = tick.tick() => {
                if !app.busy && app.overlay.is_none() && app.focus != crate::app::Focus::Commit {
                    app.queue_refresh(false);
                    watch_dirty = false;
                }
            }
            _ = background_tick.tick() => {
                if app.poll_background().await {
                    watch_dirty = false;
                    last_watch_refresh = time::Instant::now();
                }
                if watch_dirty
                    && last_watch_refresh.elapsed() >= watch_debounce
                    && !app.busy
                    && app.overlay.is_none()
                    && app.focus != crate::app::Focus::Commit
                {
                    app.queue_refresh(false);
                    watch_dirty = false;
                    last_watch_refresh = time::Instant::now();
                }
            }
            changed = watch_events.recv(), if watcher_active => {
                if changed.is_some() {
                    watch_dirty = true;
                }
            }
        }
    }
    drop(terminal);
    guard.leave()?;
    std::mem::forget(guard);
    Ok(())
}

fn start_repository_watcher(app: &mut App, tx: mpsc::Sender<()>) -> Option<RecommendedWatcher> {
    let mut watcher =
        match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.as_ref().is_ok_and(repository_event_matters) {
                let _ = tx.try_send(());
            }
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                app.output
                    .push(format!("filesystem watching unavailable: {error}"));
                return None;
            }
        };
    for view in &app.repos {
        if let Err(error) = watcher.watch(view.repo.root(), RecursiveMode::Recursive) {
            app.output.push(format!(
                "could not watch {}: {error}",
                view.repo.root().display()
            ));
            return None;
        }
    }
    Some(watcher)
}

fn repository_event_matters(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|path| {
        let components = path
            .components()
            .map(|component| component.as_os_str())
            .collect::<Vec<_>>();
        let Some(git_index) = components.iter().position(|component| *component == ".git") else {
            return true;
        };
        let Some(git_entry) = components.get(git_index + 1) else {
            return false;
        };
        matches!(
            git_entry.to_string_lossy().as_ref(),
            "index"
                | "HEAD"
                | "refs"
                | "packed-refs"
                | "MERGE_HEAD"
                | "REBASE_HEAD"
                | "CHERRY_PICK_HEAD"
                | "REVERT_HEAD"
        )
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::{
        Event,
        event::{AccessKind, ModifyKind},
    };

    use super::*;

    #[test]
    fn watcher_ignores_reads_and_git_object_churn() {
        let source_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(PathBuf::from("/repo/src/main.rs"));
        let index_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(PathBuf::from("/repo/.git/index"));
        let object_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(PathBuf::from("/repo/.git/objects/ab/cdef"));
        let read = Event::new(EventKind::Access(AccessKind::Any))
            .add_path(PathBuf::from("/repo/src/main.rs"));

        assert!(repository_event_matters(&source_change));
        assert!(repository_event_matters(&index_change));
        assert!(!repository_event_matters(&object_change));
        assert!(!repository_event_matters(&read));
    }
}
