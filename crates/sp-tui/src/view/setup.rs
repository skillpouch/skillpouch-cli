//! The setup screen: one question at a time, the answer under the cursor
//! explained beside or below it.

use super::widgets::{field, keys, spinner};
use crate::app::App;
use crate::model::Dashboard;
use crate::setup::{Mode, Setup, Step, pouches};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use sp_harness::{Harness, Kind};

/// Width of the assistant list beside the preview.
const LIST: u16 = 30;
/// Wider than this, the text column stops growing.
const MAX_WIDTH: u16 = 100;

pub fn draw(app: &App, setup: &Setup, dashboard: &Dashboard, frame: &mut Frame, area: Rect) {
    let [header, _, body, _, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(app, frame, header);
    let body = body.inner(Margin::new(2, 0));
    let body = Rect {
        width: body.width.min(MAX_WIDTH),
        ..body
    };
    let lines = match setup.step {
        Step::Pouch => pouch_step(app, setup, dashboard),
        Step::Mode => mode_step(setup, dashboard),
        Step::Assistant => return assistant_step(setup, dashboard, frame, body, footer),
        Step::Confirm => confirm_step(app, setup, dashboard),
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    draw_footer(setup, frame, footer);
}

fn draw_header(app: &App, frame: &mut Frame, area: Rect) {
    let [top, rule] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SkillPouch", theme::brand().add_modifier(Modifier::BOLD)),
            Span::styled("    Set up ", theme::meta()),
            Span::styled(app.launch.hostname.as_str(), theme::text()),
        ])),
        top.inner(Margin::new(2, 0)),
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "─".repeat(usize::from(rule.width)),
            Style::new().fg(theme::BORDER),
        )),
        rule,
    );
}

fn draw_footer(setup: &Setup, frame: &mut Frame, area: Rect) {
    let pairs: &[(&str, &str)] = match setup.step {
        _ if setup.saving => &[],
        Step::Pouch => &[
            ("↑↓", "move"),
            ("enter", "choose"),
            ("esc", "skip"),
            ("q", "quit"),
        ],
        Step::Confirm => &[("enter", "save"), ("esc", "back"), ("q", "quit")],
        _ => &[
            ("↑↓", "move"),
            ("enter", "choose"),
            ("esc", "back"),
            ("q", "quit"),
        ],
    };
    frame.render_widget(Paragraph::new(keys(pairs)), area.inner(Margin::new(2, 0)));
}

fn title(text: impl Into<String>, hint: impl Into<String>) -> Vec<Line<'static>> {
    vec![
        Line::styled(text.into(), theme::strong()),
        Line::styled(hint.into(), theme::meta()),
        Line::default(),
    ]
}

/// A choice: `› Label   detail`, highlighted under the cursor.
fn row(selected: bool, label: String, width: usize, detail: Vec<Span<'static>>) -> Line<'static> {
    let (marker, style) = if selected {
        (
            Span::styled("› ", theme::brand()),
            theme::strong().bg(theme::SELECTED_BG),
        )
    } else {
        (Span::raw("  "), theme::text())
    };
    let mut spans = vec![marker, Span::styled(format!("{label:<width$}"), style)];
    spans.push(Span::raw("   "));
    spans.extend(detail);
    Line::from(spans)
}

fn items(count: usize, more: bool) -> String {
    match (count, more) {
        (0, _) => "empty".into(),
        (1, false) => "1 item".into(),
        (count, true) => format!("{count}+ items"),
        (count, false) => format!("{count} items"),
    }
}

fn pouch_step(app: &App, setup: &Setup, dashboard: &Dashboard) -> Vec<Line<'static>> {
    let mut lines = title(
        format!("Which pouch should {} sync?", app.launch.hostname),
        "A computer syncs one pouch at a time. You can switch later on the Pouches tab.",
    );
    let choices = pouches(dashboard);
    let width = choices
        .iter()
        .map(|pouch| pouch.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(12);
    let active = dashboard.active_pouch().map(|pouch| pouch.id.as_str());
    for (index, pouch) in choices.iter().enumerate() {
        let mut detail = vec![Span::styled(
            format!("{:<10}", items(pouch.items, pouch.more)),
            theme::muted(),
        )];
        if active == Some(pouch.id.as_str()) {
            detail.push(Span::styled("synced here now", theme::accent()));
        }
        lines.push(row(index == setup.cursor, pouch.label(), width, detail));
    }
    if choices.is_empty() {
        lines.push(Line::styled(
            "No pouch can sync right now. Unlock or create one in the web app.",
            theme::warn(),
        ));
    }
    error(setup, &mut lines);
    lines
}

fn mode_step(setup: &Setup, dashboard: &Dashboard) -> Vec<Line<'static>> {
    let pouch = setup.chosen_pouch(dashboard);
    let name = pouch.map_or_else(|| "The pouch".into(), |pouch| pouch.label());
    let count = pouch.map_or_else(String::new, |pouch| items(pouch.items, pouch.more));
    let mut lines = title(
        format!("{name} already has {count}. How should this computer start?"),
        "Nothing is deleted without a backup: replaced files go to ~/.skillpouch/backups.",
    );
    let width = Mode::ALL
        .iter()
        .map(|mode| mode.label().len())
        .max()
        .unwrap_or(0);
    for (index, mode) in Mode::ALL.iter().enumerate() {
        lines.push(row(
            index == setup.cursor,
            mode.label().into(),
            width,
            vec![],
        ));
        lines.push(Line::styled(
            format!("    {}", mode.detail()),
            theme::meta(),
        ));
        lines.push(Line::default());
    }
    error(setup, &mut lines);
    lines
}

fn assistant_step(
    setup: &Setup,
    dashboard: &Dashboard,
    frame: &mut Frame,
    area: Rect,
    footer: Rect,
) {
    let name = setup
        .chosen_pouch(dashboard)
        .map_or_else(|| "the pouch".into(), |pouch| pouch.label());
    let (heading, hint) = match setup.mode {
        None => (
            format!("{name} is empty. Start it from which assistant?"),
            "Its setup is copied into the pouch. The assistant keeps working as before.",
        ),
        Some(Mode::Merge) => (
            format!("Merge {name} with which assistant?"),
            "Its items join the pouch's. Same name, different content: both are kept.",
        ),
        Some(_) => (
            format!("Replace {name} with which assistant's setup?"),
            "The pouch ends up with exactly what this assistant has.",
        ),
    };
    let heading = title(heading, hint);
    let [top, rest] = Layout::vertical([
        Constraint::Length(u16::try_from(heading.len()).unwrap_or(3)),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(heading).wrap(Wrap { trim: false }), top);

    let [list, _, preview] = Layout::horizontal([
        Constraint::Length(LIST),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .areas(rest);
    let width = usize::from(LIST).saturating_sub(12);
    let mut rows: Vec<Line> = setup
        .assistants
        .iter()
        .enumerate()
        .map(|(index, harness)| {
            row(
                index == setup.cursor,
                harness.name.into(),
                width,
                vec![Span::styled(
                    harness.items.len().to_string(),
                    theme::muted(),
                )],
            )
        })
        .collect();
    if setup.mode.is_none() {
        rows.push(row(
            setup.cursor == setup.assistants.len(),
            "Start empty".into(),
            width,
            vec![],
        ));
    }
    frame.render_widget(Paragraph::new(rows), list);

    let detail = match setup.highlighted() {
        Some(harness) => contents(harness, usize::from(preview.width)),
        None => vec![
            Line::styled("Nothing from this computer", theme::strong()),
            Line::default(),
            Line::styled(
                "The pouch starts empty. Add skills, agents and MCP servers in the web app or its marketplaces.",
                theme::meta(),
            ),
        ],
    };
    frame.render_widget(Paragraph::new(detail).wrap(Wrap { trim: false }), preview);
    draw_footer(setup, frame, footer);
}

/// What an assistant holds, kind by kind, names run together.
fn contents(harness: &Harness, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(harness.name, theme::strong()),
        Line::styled(harness.root.display().to_string(), theme::meta()),
        Line::default(),
    ];
    if harness.items.is_empty() {
        lines.push(Line::styled("Nothing to bring yet.", theme::meta()));
    }
    for kind in Kind::ALL {
        let names: Vec<&str> = harness
            .items
            .iter()
            .filter(|item| item.kind == kind)
            .map(|item| item.name.as_str())
            .collect();
        if names.is_empty() {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(kind.title(), theme::text()),
            Span::styled(format!("  {}", names.len()), theme::meta()),
        ]));
        let mut current = String::from("  ");
        for (index, name) in names.iter().enumerate() {
            let piece = if index + 1 < names.len() {
                format!("{name} · ")
            } else {
                (*name).to_string()
            };
            if current.chars().count() + piece.chars().count() > width && !current.trim().is_empty()
            {
                lines.push(Line::styled(current.trim_end().to_string(), theme::muted()));
                current = String::from("  ");
            }
            current.push_str(&piece);
        }
        lines.push(Line::styled(current, theme::muted()));
        lines.push(Line::default());
    }
    lines
}

fn confirm_step(app: &App, setup: &Setup, dashboard: &Dashboard) -> Vec<Line<'static>> {
    let pouch = setup.chosen_pouch(dashboard);
    let name = pouch.map_or_else(|| "the pouch".into(), |pouch| pouch.label());
    let assistant = setup.chosen_assistant();
    let mut lines = title(
        format!("Ready to set up {}", app.launch.hostname),
        "Check it, then press enter.",
    );
    lines.push(field(
        "Pouch",
        vec![Span::styled(name.clone(), theme::accent())],
    ));
    let (first, then) = match (setup.mode, assistant) {
        (None, Some(harness)) => (
            format!("Start from {}", harness.name),
            format!("{} go into {name}", harness.summary()),
        ),
        (None, None) => (
            "Start empty".into(),
            "Nothing on this computer changes".into(),
        ),
        (Some(Mode::UsePouch), _) => (
            "Use the pouch".into(),
            format!("This computer gets the {} in {name}", count(pouch)),
        ),
        (Some(Mode::Merge), Some(harness)) => (
            format!("Merge with {}", harness.name),
            format!("{} join the {} in {name}", harness.summary(), count(pouch)),
        ),
        (Some(_), harness) => (
            format!(
                "Replace the pouch with {}",
                harness.map_or("this computer", |h| h.name)
            ),
            harness.map_or_else(String::new, |h| {
                format!("{name} ends up with {}", h.summary())
            }),
        ),
    };
    lines.push(field(
        "First sync",
        vec![Span::styled(first, theme::text())],
    ));
    lines.push(field("", vec![Span::styled(then, theme::muted())]));
    lines.push(field(
        "Backups",
        vec![Span::styled(
            "Files that get replaced are copied to ~/.skillpouch/backups first",
            theme::muted(),
        )],
    ));
    if setup.mode == Some(Mode::UseComputer) {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!(
                "The {} in {name} are replaced for every computer that syncs it.",
                count(pouch)
            ),
            theme::warn(),
        ));
    }
    lines.push(Line::default());
    if setup.saving {
        lines.push(Line::from(vec![
            spinner(app.tick),
            Span::styled("  Saving", theme::muted()),
        ]));
    }
    error(setup, &mut lines);
    lines
}

fn count(pouch: Option<&crate::model::Pouch>) -> String {
    pouch.map_or_else(|| "items".into(), |pouch| items(pouch.items, pouch.more))
}

fn error(setup: &Setup, lines: &mut Vec<Line<'static>>) {
    if let Some(message) = &setup.error {
        lines.push(Line::default());
        lines.push(Line::styled(message.clone(), theme::error()));
    }
}

#[cfg(test)]
mod tests {
    use crate::app::tests::ready;
    use crate::model::Update;
    use crate::setup::tests::{assistants, board};
    use crate::view::tests::screen;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(app: &mut crate::app::App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn walks_through_setting_up_an_empty_pouch() {
        let mut app = ready();
        app.apply(Update::Loaded(Box::new(board())));
        app.apply(Update::Setup(assistants()));
        let text = screen(&app, 110, 30);
        for expected in [
            "Set up laptop",
            "Which pouch should laptop sync?",
            "› Work stuff",
            "12 items",
            "synced here now",
            "Pouch 2",
            "empty",
            "esc skip",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        assert!(!text.contains("Overview"), "{text}");

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        let text = screen(&app, 110, 30);
        for expected in [
            "Pouch 2 is empty. Start it from which assistant?",
            "› Claude Code",
            "Codex",
            "Start empty",
            "/home/ada/.claude",
            "Skills  2",
            "pdf-tools · reviewer",
            "MCP servers  1",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }

        press(&mut app, KeyCode::Enter);
        let text = screen(&app, 110, 30);
        for expected in [
            "Ready to set up laptop",
            "Start from Claude Code",
            "CLAUDE.md, 2 skills and 1 MCP server go into Pouch 2",
            "enter save",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
    }

    #[test]
    fn replacing_a_pouch_warns_first() {
        let mut app = ready();
        app.apply(Update::Setup(assistants()));
        press(&mut app, KeyCode::Enter);
        let text = screen(&app, 110, 30);
        for expected in [
            "Work stuff already has 12 items.",
            "Use the pouch",
            "Merge both",
            "Use this computer",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        let text = screen(&app, 110, 30);
        assert!(
            text.contains(
                "The 12 items in Work stuff are replaced for every computer that syncs it."
            ),
            "{text}"
        );
    }
}
