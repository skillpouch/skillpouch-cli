//! The five tabs. Plain text and tables, no boxes: headings, label columns
//! and a highlighted row are enough.

use super::widgets::{
    bytes, capitalised, field, heading, key_storage, list_field, meter, spinner, state_style,
};
use crate::app::{AccountAction, App, Status, Tab};
use crate::model::{Computer, Dashboard, Pouch};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState, Wrap};

/// The dashboard, or a line saying why there isn't one yet.
fn loaded<'a>(app: &'a App, frame: &mut Frame, area: Rect) -> Option<&'a Dashboard> {
    let lines = match &app.status {
        Status::Ready(dashboard) => return Some(dashboard),
        Status::Loading | Status::SignedOut => vec![Line::from(vec![
            spinner(app.tick),
            Span::styled("  Loading your account", theme::muted()),
        ])],
        Status::Failed(message) => vec![
            Line::styled("Couldn't load your account.", theme::text()),
            Line::styled(message.as_str(), theme::error()),
            Line::default(),
            Line::from(vec![
                Span::styled("r", theme::text()),
                Span::styled(" to try again", theme::meta()),
            ]),
        ],
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    None
}

fn table<'a>(rows: Vec<Row<'a>>, widths: &[Constraint], head: &[&'static str]) -> Table<'a> {
    Table::new(rows, widths.to_vec())
        .header(
            Row::new(head.iter().map(|c| Cell::from(*c)))
                .style(theme::meta())
                .bottom_margin(1),
        )
        .column_spacing(3)
        .row_highlight_style(theme::selected())
        .highlight_spacing(HighlightSpacing::Never)
}

fn render_table(frame: &mut Frame, area: Rect, table: Table, selected: usize) {
    let mut state = TableState::new().with_selected(Some(selected));
    frame.render_stateful_widget(table, area, &mut state);
}

/// The table sized to its rows, and what's left under it for details.
fn split(area: Rect, rows: usize) -> (Rect, Rect) {
    let height = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(2);
    let [top, _, bottom] = Layout::vertical([
        Constraint::Length(height),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);
    (top, bottom)
}

/// How far `computer` is with `pouch`.
fn sync(pouch: &Pouch, computer: &Computer) -> Span<'static> {
    let acked = computer
        .cursors
        .iter()
        .find(|(id, _)| id == &pouch.id)
        .map(|(_, seq)| *seq);
    match acked {
        Some(seq) if seq >= pouch.event_seq => Span::styled("up to date", theme::success()),
        Some(seq) => {
            let behind = pouch.event_seq - seq;
            let noun = if behind == 1 { "event" } else { "events" };
            Span::styled(format!("{behind} {noun} behind"), theme::warn())
        }
        None => Span::styled("not synced", theme::meta()),
    }
}

fn items(pouch: &Pouch) -> String {
    if pouch.more {
        format!("{}+", pouch.items)
    } else {
        pouch.items.to_string()
    }
}

pub fn overview(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let account = &dashboard.account;
    let [greeting, _, stats, _, details] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);

    let first = account
        .name
        .split_whitespace()
        .next()
        .unwrap_or(&account.name);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!("Welcome back, {first}"), theme::strong()),
            Line::styled(account.email.as_str(), theme::meta()),
        ]),
        greeting,
    );

    let browsers = match dashboard.browsers {
        0 => String::new(),
        1 => "and 1 browser".into(),
        n => format!("and {n} browsers"),
    };
    let columns = Layout::horizontal([Constraint::Max(22); 4]).split(stats);
    let meter_width = usize::from(columns[3].width).saturating_sub(2);
    let stat = |label: &'static str, value: Vec<Span<'static>>, below: Vec<Span<'static>>| {
        Paragraph::new(vec![
            Line::styled(label, theme::meta()),
            Line::from(value),
            Line::from(below),
        ])
    };
    let tiles = [
        stat(
            "Pouches",
            vec![
                Span::styled(dashboard.live_pouches().to_string(), theme::strong()),
                Span::styled(format!(" of {}", account.pouch_limit), theme::muted()),
            ],
            vec![],
        ),
        stat(
            "Items",
            vec![Span::styled(dashboard.items().to_string(), theme::strong())],
            vec![],
        ),
        stat(
            "Computers",
            vec![Span::styled(
                dashboard.computers.len().to_string(),
                theme::strong(),
            )],
            vec![Span::styled(browsers, theme::meta())],
        ),
        stat(
            "Storage",
            vec![
                Span::styled(bytes(account.used_bytes), theme::strong()),
                Span::styled(
                    format!(" of {}", bytes(account.quota_bytes)),
                    theme::muted(),
                ),
            ],
            meter(account.used_bytes, account.quota_bytes, meter_width),
        ),
    ];
    for (tile, area) in tiles.into_iter().zip(columns.iter()) {
        frame.render_widget(tile, *area);
    }

    let [computer, pouches] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)])
        .spacing(4)
        .areas(details);
    let this = dashboard.this_computer();
    let installed: Vec<&str> = app
        .launch
        .assistants
        .iter()
        .filter(|a| a.installed)
        .map(|a| a.name.as_str())
        .collect();
    let mut lines = vec![heading("This computer"), Line::default()];
    lines.push(field(
        "Name",
        vec![Span::styled(
            this.map_or(app.launch.hostname.as_str(), |c| c.name.as_str()),
            theme::text(),
        )],
    ));
    if let Some(this) = this {
        lines.push(field(
            "System",
            vec![Span::styled(this.platform.as_str(), theme::text())],
        ));
        lines.push(field(
            "Keys",
            vec![Span::styled(
                this.key_storage.as_deref().map_or("Unknown", key_storage),
                theme::text(),
            )],
        ));
    }
    lines.extend(list_field("Assistants", &installed));
    if let Some(date) = &account.delete_after {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("Your account will be deleted on {date}."),
            theme::warn(),
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), computer);

    let mut lines = vec![heading("Sync"), Line::default()];
    for pouch in dashboard.pouches.iter().filter(|p| p.state != "deleted") {
        let status = this.map_or(Span::styled("not synced", theme::meta()), |c| {
            sync(pouch, c)
        });
        lines.push(field(&format!("Pouch {}", pouch.number), vec![status]));
    }
    if dashboard.pouches.is_empty() {
        lines.push(Line::styled("No pouches yet.", theme::meta()));
    }
    frame.render_widget(Paragraph::new(lines), pouches);
}

pub fn pouches(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let this = dashboard.this_computer();
    let rows: Vec<Row> = dashboard
        .pouches
        .iter()
        .map(|pouch| {
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("Pouch {}", pouch.number),
                    theme::text(),
                )),
                Cell::from(items(pouch)),
                Cell::from(Span::styled(
                    pouch.updated.clone().unwrap_or_else(|| "Never".into()),
                    theme::muted(),
                )),
                Cell::from(this.map_or(Span::styled("not synced", theme::meta()), |c| {
                    sync(pouch, c)
                })),
                Cell::from(Span::styled(
                    capitalised(&pouch.state),
                    state_style(&pouch.state),
                )),
            ])
        })
        .collect();
    let (top, bottom) = split(area, rows.len());
    render_table(
        frame,
        top,
        table(
            rows,
            &[
                Constraint::Fill(1),
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Length(17),
                Constraint::Length(8),
            ],
            &["Pouch", "Items", "Updated", "This computer", "State"],
        ),
        app.cursor(Tab::Pouches),
    );

    let Some(pouch) = dashboard.pouches.get(app.cursor(Tab::Pouches)) else {
        let line = Line::styled("No pouches yet. Create one in the web app.", theme::meta());
        frame.render_widget(Paragraph::new(line), bottom);
        return;
    };
    let mut lines = vec![
        Line::styled(format!("Pouch {}", pouch.number), theme::strong()),
        Line::default(),
    ];
    for computer in &dashboard.computers {
        lines.push(field(&computer.name, vec![sync(pouch, computer)]));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "Pouch names and items are end-to-end encrypted, so they show by number here. o opens this pouch in the web app.",
        theme::meta(),
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), bottom);
}

pub fn computers(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let rows: Vec<Row> = dashboard
        .computers
        .iter()
        .map(|computer| {
            let mut name = vec![Span::styled(computer.name.as_str(), theme::text())];
            if computer.current {
                name.push(Span::styled("  this computer", theme::meta()));
            }
            Row::new(vec![
                Cell::from(Line::from(name)),
                Cell::from(Span::styled(computer.platform.as_str(), theme::muted())),
                Cell::from(computer.assistants.len().to_string()),
                Cell::from(Span::styled(
                    computer.last_seen.clone().unwrap_or_else(|| "Never".into()),
                    theme::muted(),
                )),
                Cell::from(Span::styled(
                    capitalised(&computer.state),
                    state_style(&computer.state),
                )),
            ])
        })
        .collect();
    let (top, bottom) = split(area, rows.len());
    render_table(
        frame,
        top,
        table(
            rows,
            &[
                Constraint::Fill(1),
                Constraint::Length(16),
                Constraint::Length(10),
                Constraint::Length(11),
                Constraint::Length(9),
            ],
            &["Computer", "System", "Assistants", "Last seen", "State"],
        ),
        app.cursor(Tab::Computers),
    );

    let Some(computer) = dashboard.computers.get(app.cursor(Tab::Computers)) else {
        return;
    };
    let pouch = computer
        .active_pouch
        .as_ref()
        .and_then(|id| dashboard.pouches.iter().find(|p| &p.id == id))
        .map_or("None".to_string(), |p| format!("Pouch {}", p.number));
    let mut lines = vec![
        heading(&computer.name),
        Line::default(),
        field(
            "CLI",
            vec![Span::styled(
                format!("v{}", computer.cli_version),
                theme::text(),
            )],
        ),
        field(
            "Keys",
            vec![Span::styled(
                computer
                    .key_storage
                    .as_deref()
                    .map_or("Unknown", key_storage),
                theme::text(),
            )],
        ),
        field("Pouch", vec![Span::styled(pouch, theme::text())]),
        field(
            "Connected",
            vec![Span::styled(
                computer
                    .connected
                    .clone()
                    .unwrap_or_else(|| "Unknown".into()),
                theme::text(),
            )],
        ),
    ];
    lines.extend(list_field("Assistants", &computer.assistants));
    lines.push(Line::default());
    lines.push(Line::styled(
        match dashboard.browsers {
            0 => "No trusted browsers.".to_string(),
            1 => "Your account also has 1 trusted browser.".to_string(),
            n => format!("Your account also has {n} trusted browsers."),
        },
        theme::meta(),
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), bottom);
}

pub fn assistants(app: &App, frame: &mut Frame, area: Rect) {
    let all = &app.launch.assistants;
    let found = all.iter().filter(|a| a.installed).count();
    let [intro, list] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!("{found} of {}", all.len()), theme::strong()),
                Span::styled(" assistants found on this computer", theme::muted()),
            ]),
            Line::styled(
                "SkillPouch syncs the skills, agents, MCP servers and instructions of each one it finds.",
                theme::meta(),
            ),
        ])
        .wrap(Wrap { trim: false }),
        intro,
    );
    let rows: Vec<Row> = all
        .iter()
        .map(|assistant| {
            Row::new(vec![
                Cell::from(Span::styled(
                    assistant.name.as_str(),
                    if assistant.installed {
                        theme::text()
                    } else {
                        theme::muted()
                    },
                )),
                Cell::from(if assistant.installed {
                    Span::styled("Found", theme::success())
                } else {
                    Span::styled("Not found", theme::meta())
                }),
                Cell::from(Span::styled(
                    format!("~/{}", assistant.folder),
                    Style::new().fg(theme::META),
                )),
            ])
        })
        .collect();
    render_table(
        frame,
        list,
        table(
            rows,
            &[
                Constraint::Length(18),
                Constraint::Length(10),
                Constraint::Fill(1),
            ],
            &["Assistant", "Status", "Folder"],
        ),
        app.cursor(Tab::Assistants),
    );
}

pub fn account(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let account = &dashboard.account;
    let mut storage = vec![
        Span::styled(bytes(account.used_bytes), theme::text()),
        Span::styled(
            format!(" of {}   ", bytes(account.quota_bytes)),
            theme::meta(),
        ),
    ];
    storage.extend(meter(account.used_bytes, account.quota_bytes, 16));
    let mut lines = vec![
        field(
            "Name",
            vec![Span::styled(account.name.as_str(), theme::text())],
        ),
        field(
            "Email",
            vec![Span::styled(account.email.as_str(), theme::text())],
        ),
        field(
            "Plan",
            vec![Span::styled(account.plan.as_str(), theme::accent())],
        ),
        field(
            "Pouches",
            vec![Span::styled(
                format!("{} of {}", dashboard.live_pouches(), account.pouch_limit),
                theme::text(),
            )],
        ),
        field("Storage", storage),
        field(
            "Server",
            vec![Span::styled(app.launch.server.as_str(), theme::muted())],
        ),
    ];
    if let Some(date) = &account.delete_after {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("Your account will be deleted on {date}. Cancel it in the web app."),
            theme::warn(),
        ));
    }
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let [details, _, actions] = Layout::vertical([
        Constraint::Length(height),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), details);

    let rows: Vec<Row> = AccountAction::ALL
        .iter()
        .map(|action| {
            let style = if *action == AccountAction::Logout {
                theme::error()
            } else {
                theme::text()
            };
            Row::new(vec![
                Cell::from(Span::styled(action.label(), style)),
                Cell::from(Span::styled(action.detail(), theme::meta())),
            ])
        })
        .collect();
    render_table(
        frame,
        actions,
        table(
            rows,
            &[Constraint::Length(28), Constraint::Fill(1)],
            &["Actions", ""],
        ),
        app.cursor(Tab::Account),
    );
}
