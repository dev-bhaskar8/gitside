use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    app::{App, Focus, HitRegion, Overlay, UiAction},
    config::LayoutPreference,
    model::{Change, ChangeKind},
};

// Reset delegates the canvas to the terminal profile instead of painting an
// opaque application background. This preserves Ghostty, tmux, and terminal
// theme background colors (including transparency).
const BG: Color = Color::Reset;
const PANEL: Color = Color::Reset;
const TEXT: Color = Color::Reset;
const BLUE: Color = Color::LightBlue;
const GREEN: Color = Color::LightGreen;
const RED: Color = Color::LightRed;
const ORANGE: Color = Color::LightYellow;

fn muted_style() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::DIM)
}

fn panel_border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Reset)
    }
}

fn selection_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG)), area);
    app.hits.clear();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height(area.width)),
            Constraint::Length(if area.height < 20 { 3 } else { 5 }),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, rows[0]);
    render_commit(frame, app, rows[1]);
    let compact = app.settings.layout == LayoutPreference::Compact
        || (app.settings.layout == LayoutPreference::Auto && area.width < 100);
    let force_wide = app.settings.layout == LayoutPreference::Wide;
    if app.preview.is_some() && (compact || area.width < 160) {
        render_preview(frame, app, rows[2]);
    } else if compact && !force_wide {
        render_compact_body(frame, app, rows[2]);
    } else {
        render_wide_body(frame, app, rows[2], area.width >= 160);
    }
    render_status(frame, app, rows[3]);
    if let Some(overlay) = app.overlay.clone() {
        render_overlay(frame, app, area, overlay);
    }
}

fn header_height(_width: u16) -> u16 {
    3
}

fn render_header(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let active = app.active();
    let branch = active.status.branch.head.as_deref().unwrap_or("detached");
    let sync = match (active.status.branch.ahead, active.status.branch.behind) {
        (0, 0) => String::new(),
        (ahead, behind) => format!("  ↑{ahead} ↓{behind}"),
    };
    let title = if area.width < 58 {
        format!(" {}  {}", active.repo.name(), branch)
    } else {
        format!(" Gitside  │  {}  │  {}{}", active.repo.name(), branch, sync)
    };
    frame.render_widget(
        Paragraph::new(title)
            .style(
                Style::default()
                    .fg(TEXT)
                    .bg(PANEL)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(panel_border_style(false)),
            ),
        area,
    );

    let thin_toolbar = area.width < 75;
    let push_label = app.push_control_label();
    let expanded = [
        "f Fetch".to_owned(),
        "l Pull".to_owned(),
        format!("p {push_label}"),
        "r ↻".to_owned(),
    ];
    let expanded_width = expanded
        .iter()
        .map(|label| display_width(label) as u16 + 2)
        .sum::<u16>()
        + 3;
    let full_labels = area.width >= expanded_width;
    let labels = if full_labels {
        expanded
    } else {
        [
            "f".to_owned(),
            "l".to_owned(),
            "p".to_owned(),
            "r".to_owned(),
        ]
    };
    let actions = [
        UiAction::Fetch,
        UiAction::Pull,
        UiAction::Push,
        UiAction::Refresh,
    ];
    let gap = u16::from(area.width >= 15);
    let widths = labels
        .each_ref()
        .map(|label| display_width(label) as u16 + 2);
    let total_width = widths.iter().sum::<u16>() + gap * 3;
    let mut left = area.right().saturating_sub(total_width);
    let button_y = if thin_toolbar { area.y + 1 } else { area.y };
    for (index, ((label, action), width)) in labels.into_iter().zip(actions).zip(widths).enumerate()
    {
        let rect = Rect::new(left, button_y, width, 1);
        frame.render_widget(
            Paragraph::new(format!("[{label}]"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(BLUE).bg(PANEL)),
            rect,
        );
        app.hits.push(HitRegion { rect, action });
        left = rect.right() + u16::from(index < 3) * gap;
    }
}

fn render_commit(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Commit;
    let message = if app.commit_message.is_empty() && !focused {
        Text::from(Line::from(Span::styled(
            "Message (c to edit, Ctrl+Enter to commit)",
            muted_style(),
        )))
    } else {
        Text::from(app.commit_message.clone())
    };
    let block = Block::default()
        .title(" Commit ")
        .borders(Borders::ALL)
        .border_style(panel_border_style(focused))
        .style(Style::default().bg(PANEL));
    frame.render_widget(
        Paragraph::new(message)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::Commit),
    });
    let button_width = if area.width >= 58 { 10 } else { 4 };
    let button = Rect::new(
        area.right().saturating_sub(button_width + 1),
        area.bottom().saturating_sub(2),
        button_width,
        1,
    );
    frame.render_widget(
        Paragraph::new(if button_width > 4 {
            " ✓ Commit "
        } else {
            " ✓ "
        })
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(BLUE)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        ),
        button,
    );
    app.hits.push(HitRegion {
        rect: button,
        action: UiAction::Commit,
    });
}

fn render_compact_body(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    // A narrow side pane is usually tall enough to remain useful as a stacked
    // source-control panel. Preserve Changes + Graph at every width and only
    // collapse to the focused view when vertical space is genuinely scarce.
    if area.height < 20 {
        match app.focus {
            Focus::Staged => render_changes(frame, app, area, true),
            Focus::Graph => render_graph(frame, app, area),
            Focus::Branches => render_branches(frame, app, area),
            Focus::Stashes => render_stashes(frame, app, area),
            Focus::Worktrees => render_worktrees(frame, app, area),
            Focus::GitHub => render_github(frame, app, area),
            Focus::Preview => render_preview(frame, app, area),
            _ => render_changes(frame, app, area, false),
        }
        return;
    }
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);
    if app.focus == Focus::Staged {
        render_changes(frame, app, sections[0], true);
    } else {
        render_changes(frame, app, sections[0], false);
    }
    match app.focus {
        Focus::Branches => render_branches(frame, app, sections[1]),
        Focus::Stashes => render_stashes(frame, app, sections[1]),
        Focus::Worktrees => render_worktrees(frame, app, sections[1]),
        Focus::GitHub => render_github(frame, app, sections[1]),
        _ => render_graph(frame, app, sections[1]),
    }
}

fn render_wide_body(frame: &mut Frame<'_>, app: &mut App, area: Rect, three: bool) {
    if three {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(28),
                Constraint::Percentage(34),
                Constraint::Percentage(38),
            ])
            .split(area);
        let changes = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(columns[0]);
        render_changes(frame, app, changes[0], false);
        render_changes(frame, app, changes[1], true);
        render_graph(frame, app, columns[1]);
        if app.preview.is_some() {
            render_preview(frame, app, columns[2]);
        } else if app.focus == Focus::GitHub {
            render_github(frame, app, columns[2]);
        } else if app.focus == Focus::Stashes {
            render_stashes(frame, app, columns[2]);
        } else if app.focus == Focus::Worktrees {
            render_worktrees(frame, app, columns[2]);
        } else {
            render_branches(frame, app, columns[2]);
        }
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(area);
        let left = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(columns[0]);
        render_changes(frame, app, left[0], app.focus == Focus::Staged);
        render_graph(frame, app, left[1]);
        if app.preview.is_some() {
            render_preview(frame, app, columns[1]);
        } else if app.focus == Focus::GitHub {
            render_github(frame, app, columns[1]);
        } else if app.focus == Focus::Stashes {
            render_stashes(frame, app, columns[1]);
        } else if app.focus == Focus::Worktrees {
            render_worktrees(frame, app, columns[1]);
        } else {
            render_branches(frame, app, columns[1]);
        }
    }
}

fn render_changes(frame: &mut Frame<'_>, app: &mut App, area: Rect, staged: bool) {
    let changes = if staged {
        app.active().status.staged.clone()
    } else if !app.active().status.conflicts.is_empty() {
        app.active().status.conflicts.clone()
    } else {
        app.active().status.unstaged.clone()
    };
    let focused = app.focus
        == if staged {
            Focus::Staged
        } else {
            Focus::Changes
        };
    let selected = if staged {
        app.selected_staged
    } else {
        app.selected_change
    };
    let title = if staged {
        format!(" Staged Changes ({}) ", changes.len())
    } else if !app.active().status.conflicts.is_empty() {
        format!(" Merge Changes ({}) ", changes.len())
    } else {
        format!(" Changes ({}) ", changes.len())
    };
    let items = changes
        .iter()
        .enumerate()
        .map(|(index, change)| {
            let selected_row = focused && selected == index;
            ListItem::new(change_line(change, area.width.saturating_sub(4))).style(
                if selected_row {
                    selection_style()
                } else {
                    Style::default().fg(TEXT)
                },
            )
        })
        .collect::<Vec<_>>();
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(panel_border_style(focused));
    frame.render_widget(List::new(items).block(block), area);
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(if staged {
            Focus::Staged
        } else {
            Focus::Changes
        }),
    });
    let visible = area.height.saturating_sub(2) as usize;
    for (row, _) in changes.iter().take(visible).enumerate() {
        app.hits.push(HitRegion {
            rect: Rect::new(
                area.x + 1,
                area.y + 1 + row as u16,
                area.width.saturating_sub(2),
                1,
            ),
            action: UiAction::SelectChange { staged, index: row },
        });
    }
    if area.width >= 28 {
        let action = if staged {
            UiAction::UnstageAll
        } else {
            UiAction::StageAll
        };
        let label = if staged { " − All " } else { " + All " };
        let rect = Rect::new(area.right().saturating_sub(8), area.y, 7, 1);
        frame.render_widget(Paragraph::new(label).style(Style::default().fg(BLUE)), rect);
        app.hits.push(HitRegion { rect, action });
    }
}

fn render_graph(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Graph;
    let items = app
        .active()
        .history
        .iter()
        .enumerate()
        .map(|(index, commit)| {
            let oid = commit.oid.get(..7).unwrap_or(&commit.oid);
            let decorations = if commit.decorations.is_empty() {
                String::new()
            } else {
                format!(" [{}]", commit.decorations.join(", "))
            };
            let line = format!(
                "● {} {}{}  {}",
                oid, commit.subject, decorations, commit.author
            );
            ListItem::new(line).style(if focused && index == app.selected_commit {
                selection_style()
            } else {
                Style::default().fg(TEXT)
            })
        })
        .collect::<Vec<_>>();
    let block = Block::default()
        .title(format!(" Graph ({}) ", app.active().history.len()))
        .borders(Borders::ALL)
        .border_style(panel_border_style(focused));
    frame.render_widget(List::new(items).block(block), area);
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::Graph),
    });
    for row in 0..app
        .active()
        .history
        .len()
        .min(area.height.saturating_sub(2) as usize)
    {
        app.hits.push(HitRegion {
            rect: Rect::new(
                area.x + 1,
                area.y + 1 + row as u16,
                area.width.saturating_sub(2),
                1,
            ),
            action: UiAction::SelectCommit(row),
        });
    }
}

fn render_branches(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Branches;
    let items = app
        .active()
        .branches
        .iter()
        .enumerate()
        .map(|(index, branch)| {
            let marker = if branch.current {
                "●"
            } else if branch.remote {
                "☁"
            } else {
                "○"
            };
            ListItem::new(format!("{marker} {}", branch.name)).style(
                if focused && index == app.selected_branch {
                    selection_style()
                } else if branch.current {
                    Style::default().fg(BLUE)
                } else {
                    Style::default().fg(TEXT)
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(" Branches ({}) ", app.active().branches.len()))
                .borders(Borders::ALL)
                .border_style(panel_border_style(focused)),
        ),
        area,
    );
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::Branches),
    });
    for row in 0..app
        .active()
        .branches
        .len()
        .min(area.height.saturating_sub(2) as usize)
    {
        app.hits.push(HitRegion {
            rect: Rect::new(
                area.x + 1,
                area.y + 1 + row as u16,
                area.width.saturating_sub(2),
                1,
            ),
            action: UiAction::SelectBranch(row),
        });
    }
}

fn render_stashes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Stashes;
    let narrow = area.width < 55;
    let items = app
        .active()
        .stashes
        .iter()
        .enumerate()
        .map(|(index, stash)| {
            ListItem::new(format!("◇ {}  {}", stash.reference, stash.subject)).style(
                if focused && index == app.selected_stash {
                    selection_style()
                } else {
                    Style::default().fg(TEXT)
                },
            )
        })
        .collect::<Vec<_>>();
    let title = if narrow {
        format!(" Stashes ({}) ", app.active().stashes.len())
    } else {
        format!(
            " Stashes ({}) · A apply · P pop · X drop ",
            app.active().stashes.len()
        )
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(panel_border_style(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let list_area = if narrow {
        let (hint, hint_height) = if inner.width >= 25 {
            (Text::from("A apply · P pop · X drop"), 1)
        } else if inner.width >= 15 {
            (
                Text::from(vec![Line::from("A apply · P pop"), Line::from("X drop")]),
                2,
            )
        } else {
            (
                Text::from(vec![
                    Line::from("A apply"),
                    Line::from("P pop"),
                    Line::from("X drop"),
                ]),
                3,
            )
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hint_height), Constraint::Min(0)])
            .split(inner);
        frame.render_widget(Paragraph::new(hint).style(muted_style()), rows[0]);
        rows[1]
    } else {
        inner
    };
    frame.render_widget(List::new(items), list_area);
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::Stashes),
    });
    for row in 0..app.active().stashes.len().min(list_area.height as usize) {
        app.hits.push(HitRegion {
            rect: Rect::new(list_area.x, list_area.y + row as u16, list_area.width, 1),
            action: UiAction::SelectStash(row),
        });
    }
}

fn render_worktrees(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Worktrees;
    let items = app
        .active()
        .worktrees
        .iter()
        .enumerate()
        .map(|(index, worktree)| {
            let branch = worktree.branch.as_deref().unwrap_or("detached");
            let flags = if worktree.locked {
                " locked"
            } else if worktree.prunable {
                " prunable"
            } else {
                ""
            };
            ListItem::new(format!(
                "▣ {}  {}{}",
                worktree.path.display(),
                branch,
                flags
            ))
            .style(if focused && index == app.selected_worktree {
                selection_style()
            } else {
                Style::default().fg(TEXT)
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    " Worktrees ({}) · X remove ",
                    app.active().worktrees.len()
                ))
                .borders(Borders::ALL)
                .border_style(panel_border_style(focused)),
        ),
        area,
    );
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::Worktrees),
    });
    for row in 0..app
        .active()
        .worktrees
        .len()
        .min(area.height.saturating_sub(2) as usize)
    {
        app.hits.push(HitRegion {
            rect: Rect::new(
                area.x + 1,
                area.y + 1 + row as u16,
                area.width.saturating_sub(2),
                1,
            ),
            action: UiAction::SelectWorktree(row),
        });
    }
}

fn render_github(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::GitHub;
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::GitHub),
    });
    if !app.active().github_available {
        frame.render_widget(
            Paragraph::new("GitHub CLI is unavailable.\n\nInstall `gh` and run `gh auth login`.")
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(" GitHub ")
                        .borders(Borders::ALL)
                        .border_style(panel_border_style(focused)),
                ),
            area,
        );
        return;
    }
    let items: Vec<ListItem<'_>> = if app.github_show_issues {
        app.active()
            .issues
            .iter()
            .enumerate()
            .map(|(index, issue)| {
                ListItem::new(format!(
                    "#{} {}  @{}",
                    issue.number, issue.title, issue.author
                ))
                .style(if focused && index == app.selected_github {
                    selection_style()
                } else {
                    Style::default().fg(TEXT)
                })
            })
            .collect()
    } else {
        app.active()
            .pull_requests
            .iter()
            .enumerate()
            .map(|(index, pr)| {
                let draft = if pr.is_draft { " [draft]" } else { "" };
                ListItem::new(format!(
                    "#{} {}{}  {} → {}",
                    pr.number, pr.title, draft, pr.head, pr.base
                ))
                .style(if focused && index == app.selected_github {
                    selection_style()
                } else {
                    Style::default().fg(TEXT)
                })
            })
            .collect()
    };
    let mode = if app.github_show_issues {
        "Issues"
    } else {
        "Pull Requests"
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(" GitHub · {mode} (i to switch) "))
                .borders(Borders::ALL)
                .border_style(panel_border_style(focused)),
        ),
        area,
    );
    let count = if app.github_show_issues {
        app.active().issues.len()
    } else {
        app.active().pull_requests.len()
    };
    for row in 0..count.min(area.height.saturating_sub(2) as usize) {
        app.hits.push(HitRegion {
            rect: Rect::new(
                area.x + 1,
                area.y + 1 + row as u16,
                area.width.saturating_sub(2),
                1,
            ),
            action: if app.github_show_issues {
                UiAction::SelectIssue(row)
            } else {
                UiAction::SelectPullRequest(row)
            },
        });
    }
}

fn render_preview(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(preview) = app.preview.as_ref() else {
        return;
    };
    let lines = preview
        .body
        .lines()
        .skip(preview.scroll as usize)
        .map(|line| {
            let style = if line.starts_with('+') && !line.starts_with("+++") {
                Style::default().fg(GREEN)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Style::default().fg(RED)
            } else if line.starts_with("@@") {
                Style::default().fg(BLUE)
            } else if line.starts_with("diff ") || line.starts_with("commit ") {
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT)
            };
            Line::styled(line.to_owned(), style)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(format!(
                    " {}  [j/k hunk · Space stage · Esc close · e editor] ",
                    preview.title
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BLUE)),
        ),
        area,
    );
    app.hits.push(HitRegion {
        rect: area,
        action: UiAction::Focus(Focus::Preview),
    });
}

fn render_status(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let help = contextual_footer_hint(
        app.focus,
        area.width,
        !app.active().status.conflicts.is_empty(),
        app.active().status.operation.is_some(),
    );
    let help_width = display_width(help);
    let available = area
        .width
        .saturating_sub(help_width as u16)
        .saturating_sub(1) as usize;
    let status = truncate_to_width(&app.status_line, available);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {status}"), muted_style().bg(PANEL)),
            Span::styled(help, Style::default().fg(TEXT).bg(PANEL)),
        ]))
        .style(Style::default().bg(PANEL)),
        area,
    );
    let help_x = area.right().saturating_sub(help_width as u16);
    let help_rect = Rect::new(help_x, area.y, 7.min(area.width), 1);
    app.hits.push(HitRegion {
        rect: help_rect,
        action: UiAction::ToggleHelp,
    });
}

fn contextual_footer_hint(
    focus: Focus,
    width: u16,
    has_conflicts: bool,
    has_operation: bool,
) -> &'static str {
    if width < 18 {
        return if focus == Focus::Commit {
            " F1 Help"
        } else {
            " ? Help"
        };
    }
    if width < 24 {
        return if focus == Focus::Commit {
            " F1 Help · More"
        } else {
            " ? Help · More"
        };
    }
    if width < 50 {
        return match focus {
            Focus::Commit => " F1 Help · Commit",
            Focus::Changes if has_conflicts => " ? Help · O/I/B Resolve",
            Focus::Changes if has_operation => " ? Help · C Continue",
            Focus::Changes => " ? Help · Space Stage",
            Focus::Staged => " ? Help · Space Unstage",
            Focus::Graph => " ? Help · Enter View",
            Focus::Branches => " ? Help · Enter Switch",
            Focus::Stashes => " ? Help · A Apply",
            Focus::Worktrees => " ? Help · X Remove",
            Focus::GitHub => " ? Help · Enter View",
            Focus::Preview => " ? Help · Esc Close",
        };
    }
    match focus {
        Focus::Commit => " F1 Help · Ctrl+Enter Commit · Esc Done",
        Focus::Changes if has_conflicts => " ? Help · O Current · I Incoming · B Both",
        Focus::Changes if has_operation => " ? Help · C Continue · S Skip · A Abort",
        Focus::Changes => " ? Help · Space Stage · e Diff · E Lines",
        Focus::Staged => " ? Help · Space Unstage · e Diff · E Lines",
        Focus::Graph => " ? Help · Enter View · y Pick · v Revert",
        Focus::Branches => " ? Help · Enter Switch · n New · x Delete",
        Focus::Stashes => " ? Help · Enter View · A Apply · P Pop",
        Focus::Worktrees => " ? Help · w Add · X Remove",
        Focus::GitHub => " ? Help · Enter View · i Type · o Open",
        Focus::Preview => " ? Help · j/k Hunk · Space Stage · Esc Close",
    }
}

fn display_width(value: &str) -> usize {
    value.chars().filter_map(UnicodeWidthChar::width).sum()
}

fn truncate_to_width(value: &str, max_width: usize) -> String {
    let mut width = 0;
    value
        .chars()
        .take_while(|character| {
            let character_width = UnicodeWidthChar::width(*character).unwrap_or(0);
            if width + character_width > max_width {
                false
            } else {
                width += character_width;
                true
            }
        })
        .collect()
}

fn render_overlay(frame: &mut Frame<'_>, app: &mut App, area: Rect, overlay: Overlay) {
    let popup = if matches!(overlay, Overlay::Search { .. }) {
        centered_sized_rect(
            area.width.saturating_sub(4).clamp(1, 72),
            area.height.saturating_sub(2).clamp(1, 12),
            area,
        )
    } else {
        centered_rect(
            if area.width < 70 { 90 } else { 60 },
            if area.height < 24 { 85 } else { 60 },
            area,
        )
    };
    frame.render_widget(Clear, popup);
    if let Overlay::Help { scroll, .. } = overlay {
        render_help_overlay(frame, app, popup, scroll);
        return;
    }
    let (title, body, border) = match overlay {
        Overlay::Help { .. } => unreachable!("help overlays render separately"),
        Overlay::Confirm { prompt, .. } => (" Confirm ".to_owned(), prompt, ORANGE),
        Overlay::Message { title, body } => {
            (" Message ".to_owned(), format!("{title}\n\n{body}"), RED)
        }
        Overlay::Prompt {
            title,
            label,
            value,
            ..
        } => (
            format!(" {title} "),
            format!("{label}\n\n{value}█\n\nEnter to confirm · Esc to cancel"),
            BLUE,
        ),
        Overlay::Search { value } => (
            " Search focused view ".to_owned(),
            format!(
                "Find files, commits, branches, or items in the current panel.\n\n/{value}█\n\nEnter to find · Esc to cancel · N finds next"
            ),
            BLUE,
        ),
    };
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(PANEL)),
        ),
        popup,
    );
    app.hits.push(HitRegion {
        rect: popup,
        action: UiAction::CloseOverlay,
    });
}

fn render_help_overlay(frame: &mut Frame<'_>, app: &mut App, popup: Rect, requested_scroll: u16) {
    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BLUE))
        .style(Style::default().bg(PANEL));
    let inner = block.inner(popup);
    let body = wrap_help_text(
        &help_text(
            app.focus,
            !app.active().status.conflicts.is_empty(),
            app.active().status.operation.is_some(),
        ),
        inner.width,
    );
    let line_count = body.lines().count();
    let paragraph = Paragraph::new(body).block(block);
    let max_scroll = line_count
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16;
    let scroll = requested_scroll.min(max_scroll);

    frame.render_widget(paragraph.scroll((scroll, 0)), popup);

    if max_scroll > 0 {
        let first_line = usize::from(scroll) + 1;
        let last_line = (usize::from(scroll) + inner.height as usize).min(line_count);
        let position = format!(
            " {}{} {first_line}–{last_line}/{line_count} {}{} ",
            if scroll > 0 { "↑ " } else { "" },
            if scroll > 0 { "more ·" } else { "" },
            if scroll < max_scroll { "· more" } else { "" },
            if scroll < max_scroll { " ↓" } else { "" },
        );
        let indicator_width = position.chars().count().min(popup.width as usize) as u16;
        let indicator = Rect::new(
            popup.right().saturating_sub(indicator_width + 2),
            popup.bottom().saturating_sub(1),
            indicator_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(position)
                .alignment(Alignment::Center)
                .style(Style::default().fg(BLUE).bg(PANEL)),
            indicator,
        );

        render_offset_scrollbar(frame, popup, scroll, max_scroll);
    }

    if let Some(Overlay::Help {
        scroll: live_scroll,
        max_scroll: live_max_scroll,
    }) = &mut app.overlay
    {
        *live_scroll = scroll;
        *live_max_scroll = max_scroll;
    }
    app.hits.push(HitRegion {
        rect: popup,
        action: UiAction::CloseOverlay,
    });
}

fn render_offset_scrollbar(frame: &mut Frame<'_>, area: Rect, scroll: u16, max_scroll: u16) {
    let track = area.inner(Margin {
        horizontal: 0,
        vertical: 1,
    });
    let Some(thumb_y) = scrollbar_thumb_row(track, scroll, max_scroll) else {
        return;
    };
    let x = track.right().saturating_sub(1);
    for y in track.top()..track.bottom() {
        frame.buffer_mut().set_string(x, y, "│", muted_style());
    }
    frame
        .buffer_mut()
        .set_string(x, thumb_y, "█", Style::default().fg(BLUE));
}

fn scrollbar_thumb_row(track: Rect, scroll: u16, max_scroll: u16) -> Option<u16> {
    if track.is_empty() || max_scroll == 0 {
        return None;
    }
    let travel = u32::from(track.height.saturating_sub(1));
    let numerator = u32::from(scroll.min(max_scroll)) * travel + u32::from(max_scroll) / 2;
    let offset = numerator / u32::from(max_scroll);
    Some(track.y + offset as u16)
}

fn wrap_help_text(value: &str, width: u16) -> String {
    let max_width = usize::from(width.max(1));
    let mut output = Vec::new();
    for line in value.split('\n') {
        if line.is_empty() {
            output.push(String::new());
            continue;
        }
        let indent: String = line
            .chars()
            .take_while(|character| character.is_whitespace())
            .take(max_width.saturating_sub(1))
            .collect();
        let mut remaining = line.to_owned();
        while display_width(&remaining) > max_width {
            let mut prefix_end = 0;
            let mut prefix_width = 0;
            let mut last_break = None;
            for (index, character) in remaining.char_indices() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if prefix_width + character_width > max_width {
                    break;
                }
                prefix_width += character_width;
                prefix_end = index + character.len_utf8();
                if character.is_whitespace() && index > indent.len() {
                    last_break = Some(index);
                }
            }
            let break_at = last_break.unwrap_or(prefix_end);
            output.push(remaining[..break_at].trim_end().to_owned());
            let tail = remaining[break_at..].trim_start();
            remaining = format!("{indent}{tail}");
        }
        output.push(remaining);
    }
    output.join("\n")
}

fn help_text(focus: Focus, has_conflicts: bool, has_operation: bool) -> String {
    let (context_title, context) = match focus {
        Focus::Commit => (
            "Commit — current panel",
            "  Type                 Edit message\n  Up/Down              Previous/next message\n  Ctrl+Enter           Commit\n  Ctrl+Shift+Enter     Amend commit\n  Ctrl+Alt+Enter       Commit with sign-off\n  Ctrl+Shift+Alt+Enter Amend with sign-off\n  F1                   Help\n  Esc                  Leave message\n  Tab                  Next panel",
        ),
        Focus::Changes if has_conflicts => (
            "Merge Changes — current panel",
            "  j/k or arrows  Move\n  Enter          Preview conflict\n  O              Accept current file\n  I              Accept incoming file\n  B              Accept both sides\n  C / S / A      Continue/skip/abort operation",
        ),
        Focus::Changes if has_operation => (
            "Git operation — current panel",
            "  C / S / A      Continue/skip/abort operation\n  Enter          Preview selected change",
        ),
        Focus::Changes => (
            "Changes — current panel",
            "  j/k or arrows  Move\n  Space          Stage file\n  Enter          Preview diff\n  a              Stage all\n  d              Discard\n  e              External old/new difftool\n  E              Interactive line staging\n  o              Open working file",
        ),
        Focus::Staged => (
            "Staged — current panel",
            "  j/k or arrows  Move\n  Space          Unstage file\n  Enter          Preview diff\n  u              Unstage all\n  e              External old/new difftool\n  E              Interactive line unstaging\n  o              Open working file",
        ),
        Focus::Graph => (
            "Graph — current panel",
            "  j/k or arrows  Move\n  Enter          View commit\n  y / v / t      Cherry-pick/revert/tag",
        ),
        Focus::Branches => (
            "Branches — current panel",
            "  j/k or arrows  Move\n  Enter          Switch branch\n  n / x          Create/delete\n  m / R          Merge/rebase\n  w              Add worktree",
        ),
        Focus::Stashes => (
            "Stashes — current panel",
            "  j/k or arrows  Move\n  Enter          Preview stash\n  A / P / X      Apply/pop/drop",
        ),
        Focus::Worktrees => (
            "Worktrees — current panel",
            "  j/k or arrows  Move\n  X              Remove worktree",
        ),
        Focus::GitHub => (
            "GitHub — current panel",
            "  j/k or arrows  Move\n  Enter          View PR/issue\n  i / o          Switch type/open web\n  C / K          Checkout PR/view checks",
        ),
        Focus::Preview => (
            "Preview — current panel",
            "  j/k or arrows  Select hunk\n  Space          Stage/unstage hunk\n  Esc            Close preview",
        ),
    };

    format!(
        "{context_title}\n{context}\n\nAll shortcuts\n\nNavigation\n  j/k or arrows  Move\n  Page Up/Down   Move 10 items\n  Home/End       First/last item\n  Tab/Shift+Tab  Next/previous panel\n  Enter          Open/activate\n  [ / ]          Previous/next repository\n  /              Search focused view\n  N              Next search match\n  ? / F1         Open/close help\n  q / Ctrl+C     Quit\n\nChanges\n  Space          Stage/unstage file or hunk\n  a / u          Stage/unstage all\n  d              Discard (confirmation)\n  e              External old/new difftool\n  E              Interactive line staging\n\nRepository\n  c                    Commit message\n  Up/Down              Previous/next message\n  Ctrl+Enter           Commit\n  Ctrl+Shift+Enter     Amend commit\n  Ctrl+Alt+Enter       Commit with sign-off\n  Ctrl+Shift+Alt+Enter Amend with sign-off\n  U                    Undo last commit\n  f / l / p            Fetch/pull/push\n  L                    Pull with rebase\n  P / T                Push to/pull from target\n  F                    Force push with lease\n  D                    Git diagnostics\n  s / z                Create/list stashes\n  W                    Worktree list\n  r                    Refresh\n\nBranches\n  n / x          Create/delete\n  m / R          Merge/rebase\n  w              Add worktree\n\nStashes\n  A / P / X      Apply/pop/drop\n\nGraph\n  y / v / t      Cherry-pick/revert/tag\n\nGit operations\n  C / S / A      Continue/skip/abort\n\nGitHub\n  Enter          View PR/issue\n  i / o          Switch type/open web\n  C / K          Checkout PR/view checks\n\nHelp scrolling\n  j/k or arrows  Scroll one line\n  Page Up/Down   Scroll ten lines\n  Home/End       Top/bottom\n  Mouse wheel    Scroll\n\nPress Esc, Enter, ?, or F1 to close."
    )
}

fn change_line(change: &Change, width: u16) -> Line<'static> {
    let color = match change.kind {
        ChangeKind::Added | ChangeKind::Untracked => GREEN,
        ChangeKind::Deleted => RED,
        ChangeKind::Conflicted => ORANGE,
        _ => TEXT,
    };
    let path = change.path.to_string_lossy();
    let max = width.saturating_sub(4) as usize;
    let display = truncate_middle(&path, max);
    Line::from(vec![
        Span::styled(
            format!("{} ", change.kind.badge()),
            Style::default().fg(color),
        ),
        Span::raw(display),
    ])
}

fn truncate_middle(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    if max < 4 {
        return value.chars().take(max).collect();
    }
    let left = (max - 1) / 2;
    let right = max - left - 1;
    let beginning: String = value.chars().take(left).collect();
    let ending: String = value
        .chars()
        .rev()
        .take(right)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{beginning}…{ending}")
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
        .inner(Margin {
            horizontal: 0,
            vertical: 0,
        })
}

fn centered_sized_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend};

    use crate::config::{Cli, Settings};

    #[test]
    fn middle_truncation_preserves_ends() {
        assert_eq!(
            truncate_middle("src/components/panel.rs", 12),
            "src/c…nel.rs"
        );
        assert_eq!(truncate_middle("short", 12), "short");
    }

    #[tokio::test]
    async fn clicking_empty_change_panel_space_moves_focus_from_commit() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.focus = Focus::Commit;
        let mut terminal = Terminal::new(TestBackend::new(50, 40)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        let panel = app
            .hits
            .iter()
            .find(|hit| {
                hit.rect.height > 1 && matches!(hit.action, UiAction::Focus(Focus::Changes))
            })
            .unwrap()
            .rect;
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((panel.x + 2, panel.bottom().saturating_sub(2)))
                .unwrap()
                .bg,
            Color::Reset
        );
        let inactive_border = terminal
            .backend()
            .buffer()
            .cell((panel.x, panel.y + 1))
            .unwrap();
        assert_eq!(inactive_border.fg, Color::Reset);
        assert!(!inactive_border.modifier.contains(Modifier::DIM));
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: panel.x + panel.width / 2,
            row: panel.bottom().saturating_sub(2),
            modifiers: KeyModifiers::NONE,
        })
        .await;

        assert_eq!(app.focus, Focus::Changes);
    }

    #[tokio::test]
    async fn empty_staged_tab_stop_remains_visibly_focused_in_compact_layout() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        assert!(app.active().status.staged.is_empty());
        app.focus = Focus::Staged;
        let mut terminal = Terminal::new(TestBackend::new(50, 40)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        let panel = app
            .hits
            .iter()
            .find(|hit| hit.rect.height > 1 && matches!(hit.action, UiAction::Focus(Focus::Staged)))
            .expect("the empty staged panel should be visible")
            .rect;
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((panel.x, panel.y + 1))
                .unwrap()
                .fg,
            BLUE
        );
    }

    #[tokio::test]
    async fn narrow_stash_panel_wraps_actions_inside_the_border() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.focus = Focus::Stashes;
        let mut terminal = Terminal::new(TestBackend::new(24, 40)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        let output: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(output.contains("Stashes (0)"));
        assert!(output.contains("A apply"));
        assert!(output.contains("P pop"));
        assert!(output.contains("X drop"));
        assert!(
            app.hits
                .iter()
                .any(|hit| matches!(hit.action, UiAction::Fetch))
        );
        assert!(
            app.hits
                .iter()
                .any(|hit| matches!(hit.action, UiAction::Pull))
        );
        assert!(
            app.hits
                .iter()
                .any(|hit| matches!(hit.action, UiAction::Push))
        );
    }

    #[tokio::test]
    async fn outlined_remote_buttons_fit_supported_widths() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.active_mut().status.branch.upstream = Some("origin/main".into());

        for width in [14, 20, 24, 34, 50, 74, 100] {
            let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let buttons = app
                .hits
                .iter()
                .filter(|hit| {
                    matches!(
                        hit.action,
                        UiAction::Fetch | UiAction::Pull | UiAction::Push | UiAction::Refresh
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(buttons.len(), 4, "width {width}");
            for button in &buttons {
                assert_eq!(button.rect.height, 1, "width {width}");
                assert!(button.rect.right() <= width, "width {width}");
                let top_left = terminal
                    .backend()
                    .buffer()
                    .cell((button.rect.x, button.rect.y))
                    .unwrap();
                assert_eq!(top_left.symbol(), "[", "width {width}");
                assert_eq!(top_left.bg, Color::Reset, "width {width}");
            }
            for (index, button) in buttons.iter().enumerate() {
                for other in buttons.iter().skip(index + 1) {
                    assert!(
                        button.rect.right() <= other.rect.x || other.rect.right() <= button.rect.x,
                        "button hit regions overlap at width {width}"
                    );
                }
            }

            let output: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            if width >= 34 {
                assert!(output.contains("[f Fetch]"), "width {width}");
                assert!(output.contains("[l Pull]"), "width {width}");
                assert!(output.contains("[p Push]"), "width {width}");
                assert!(output.contains("[r ↻]"), "width {width}");
            } else {
                assert!(output.contains("[f]"), "width {width}");
                assert!(output.contains("[l]"), "width {width}");
                assert!(output.contains("[p]"), "width {width}");
                assert!(output.contains("[r]"), "width {width}");
            }
        }
    }

    #[tokio::test]
    async fn existing_push_control_adapts_to_publish_and_sync_states() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(50, 20)).unwrap();

        app.active_mut().status.branch.upstream = None;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("[p Publish]")
        );

        let branch = &mut app.active_mut().status.branch;
        branch.upstream = Some("origin/main".into());
        branch.ahead = 2;
        branch.behind = 1;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("[p Sync]")
        );

        app.active_mut().status.branch.behind = 0;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("[p Push]")
        );
    }

    #[tokio::test]
    async fn search_overlay_stays_compact_in_a_tall_narrow_pane() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.overlay = Some(Overlay::Search {
            value: String::new(),
        });
        let mut terminal = Terminal::new(TestBackend::new(32, 45)).unwrap();

        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        let popup = app
            .hits
            .iter()
            .find(|hit| matches!(hit.action, UiAction::CloseOverlay))
            .expect("search should register its popup")
            .rect;
        assert_eq!(popup.width, 28);
        assert_eq!(popup.height, 12);
        assert!(
            popup.y > 10,
            "search should be centered instead of filling the pane"
        );
    }

    #[tokio::test]
    async fn help_starts_with_context_and_scrolls_to_complete_reference() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.focus = Focus::Branches;
        app.overlay = Some(Overlay::Help {
            scroll: 0,
            max_scroll: 0,
        });
        let mut terminal = Terminal::new(TestBackend::new(50, 20)).unwrap();

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let popup = app
            .hits
            .iter()
            .find(|hit| matches!(hit.action, UiAction::CloseOverlay))
            .expect("help should register its popup")
            .rect;
        let track = popup.inner(Margin {
            horizontal: 0,
            vertical: 1,
        });
        let track_x = track.right() - 1;
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((track_x, track.top()))
                .unwrap()
                .symbol(),
            "█"
        );
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((track_x, track.bottom() - 1))
                .unwrap()
                .symbol(),
            "│"
        );
        let first_page: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(first_page.contains("Branches — current panel"));
        assert!(first_page.contains("more"));
        let Some(Overlay::Help { scroll, max_scroll }) = &app.overlay else {
            panic!("help should remain open");
        };
        assert_eq!(*scroll, 0);
        assert!(*max_scroll > 0);

        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
            .await;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((track_x, track.top()))
                .unwrap()
                .symbol(),
            "│"
        );
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((track_x, track.bottom() - 1))
                .unwrap()
                .symbol(),
            "█"
        );
        let last_page: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(last_page.contains("Press Esc, Enter, ?, or F1 to close."));
        let Some(Overlay::Help { scroll, max_scroll }) = &app.overlay else {
            panic!("help should remain open");
        };
        assert_eq!(scroll, max_scroll);
        let max_scroll = *max_scroll;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 25,
            row: 10,
            modifiers: KeyModifiers::NONE,
        })
        .await;
        let Some(Overlay::Help { scroll, .. }) = &app.overlay else {
            panic!("help should remain open");
        };
        assert_eq!(*scroll, max_scroll.saturating_sub(3));
    }

    #[tokio::test]
    async fn commit_editor_opens_contextual_help_without_stealing_punctuation() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.focus = Focus::Commit;

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.commit_message, "?");
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL))
            .await;
        assert_eq!(app.commit_message, "?");

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .await;
        assert!(matches!(app.overlay, Some(Overlay::Help { .. })));

        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let output: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(output.contains("Commit — current panel"));
        assert!(output.contains("Ctrl+Shift+Enter"));
        assert!(output.contains("Ctrl+Alt+Enter"));

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .await;
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn conflicts_replace_changes_with_contextual_resolution_help() {
        let cli = Cli::try_parse_from(["gitside", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();
        app.focus = Focus::Changes;
        app.active_mut().status.conflicts = vec![Change {
            path: "conflict.txt".into(),
            original_path: None,
            kind: ChangeKind::Conflicted,
            staged: false,
        }];
        let mut terminal = Terminal::new(TestBackend::new(50, 30)).unwrap();

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let output: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(output.contains("Merge Changes (1)"));
        assert!(output.contains("O Current"));

        app.overlay = Some(Overlay::Help {
            scroll: 0,
            max_scroll: 0,
        });
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let output: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(output.contains("Merge Changes — current panel"));
        assert!(output.contains("Accept incoming file"));
    }

    #[test]
    fn offset_scrollbar_maps_the_full_scroll_range_to_the_track() {
        let track = Rect::new(8, 4, 1, 10);
        assert_eq!(scrollbar_thumb_row(track, 0, 10), Some(4));
        assert_eq!(scrollbar_thumb_row(track, 5, 10), Some(9));
        assert_eq!(scrollbar_thumb_row(track, 10, 10), Some(13));
        assert_eq!(scrollbar_thumb_row(track, 20, 10), Some(13));
        assert_eq!(scrollbar_thumb_row(track, 0, 0), None);
        assert_eq!(scrollbar_thumb_row(Rect::ZERO, 0, 10), None);
    }

    #[test]
    fn footer_hints_are_contextual_and_unicode_truncation_is_safe() {
        assert_eq!(
            contextual_footer_hint(Focus::Changes, 40, false, false),
            " ? Help · Space Stage"
        );
        assert_eq!(
            contextual_footer_hint(Focus::Graph, 80, false, false),
            " ? Help · Enter View · y Pick · v Revert"
        );
        assert_eq!(
            contextual_footer_hint(Focus::Commit, 80, false, false),
            " F1 Help · Ctrl+Enter Commit · Esc Done"
        );
        assert!(!contextual_footer_hint(Focus::Changes, 40, false, false).contains("Tab"));
        assert_eq!(truncate_to_width("répo 🚀", 6), "répo ");
        assert_eq!(truncate_to_width("répo 🚀", 7), "répo 🚀");

        let wrapped = wrap_help_text("  Space          Stage file", 25);
        assert_eq!(wrapped, "  Space          Stage\n  file");
        assert!(wrapped.lines().all(|line| display_width(line) <= 25));
    }
}
