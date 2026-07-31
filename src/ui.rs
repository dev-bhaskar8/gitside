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
        format!(
            " Sourcepane  │  {}  │  {}{}",
            active.repo.name(),
            branch,
            sync
        )
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
    let full_labels = area.width >= 34;
    let labels = if full_labels {
        ["Fetch", "Pull", "Push", "↻"]
    } else {
        ["F", "L", "P", "↻"]
    };
    let actions = [
        UiAction::Fetch,
        UiAction::Pull,
        UiAction::Push,
        UiAction::Refresh,
    ];
    let gap = u16::from(area.width >= 15);
    let widths = labels.map(|label| label.chars().count() as u16 + 2);
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
    let help = contextual_footer_hint(app.focus, area.width);
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

fn contextual_footer_hint(focus: Focus, width: u16) -> &'static str {
    if width < 18 {
        return " ? Help";
    }
    if width < 24 {
        return " ? Help · More";
    }
    if width < 50 {
        return match focus {
            Focus::Commit => " ? Help · Commit",
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
        Focus::Commit => " ? Help · Ctrl+Enter Commit · Esc Done",
        Focus::Changes => " ? Help · Space Stage · Enter Diff",
        Focus::Staged => " ? Help · Space Unstage · Enter Diff",
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
    let popup = centered_rect(
        if area.width < 70 { 90 } else { 60 },
        if area.height < 24 { 85 } else { 60 },
        area,
    );
    frame.render_widget(Clear, popup);
    let (title, body, border) = match overlay {
        Overlay::Help => (
            " Help ".to_owned(),
            "Navigation\n  j/k or arrows  Move\n  Tab            Next panel\n  Enter          Open/activate\n  [ / ]          Previous/next repository\n\nChanges\n  Space          Stage/unstage file or hunk\n  a / u          Stage/unstage all\n  d              Discard (confirmation)\n  e              External editor\n\nRepository\n  c              Commit message\n  Ctrl+Enter     Commit\n  f/l/p          Fetch/pull/push\n  s/z            Create/list stashes\n  W              Worktree list\n  r              Refresh\n\nBranches\n  n/x            Create/delete\n  m/R            Merge/rebase\n  w              Add worktree\n\nStashes\n  A/P/X          Apply/pop/drop\n\nGraph\n  y/v/t          Cherry-pick/revert/tag\n\nGitHub\n  Enter          View PR/issue\n  i/o            Switch type/open web\n  C/K            Checkout PR/view checks\n\nPress Esc, Enter, or ? to close."
                .to_owned(),
            BLUE,
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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
        let cli = Cli::try_parse_from(["sourcepane", "."]).unwrap();
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
        let cli = Cli::try_parse_from(["sourcepane", "."]).unwrap();
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
        let cli = Cli::try_parse_from(["sourcepane", "."]).unwrap();
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
        let cli = Cli::try_parse_from(["sourcepane", "."]).unwrap();
        let mut app = App::new(cli, Settings::default()).await.unwrap();

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
        }
    }

    #[test]
    fn footer_hints_are_contextual_and_unicode_truncation_is_safe() {
        assert_eq!(
            contextual_footer_hint(Focus::Changes, 40),
            " ? Help · Space Stage"
        );
        assert_eq!(
            contextual_footer_hint(Focus::Graph, 80),
            " ? Help · Enter View · y Pick · v Revert"
        );
        assert!(!contextual_footer_hint(Focus::Changes, 40).contains("Tab"));
        assert_eq!(truncate_to_width("répo 🚀", 6), "répo ");
        assert_eq!(truncate_to_width("répo 🚀", 7), "répo 🚀");
    }
}
