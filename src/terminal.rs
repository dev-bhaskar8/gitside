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
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::time;

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
    let mut tick = time::interval(Duration::from_millis(app.settings.refresh_ms.max(250)));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

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
                    app.refresh().await;
                }
            }
        }
    }
    drop(terminal);
    guard.leave()?;
    std::mem::forget(guard);
    Ok(())
}
