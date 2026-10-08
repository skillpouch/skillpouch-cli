//! Plays the login-screen mascot: `cargo run -p sp-tui --example mascot [idle|busy|sad]`.
//! `q` quits.

use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use sp_tui::mascot::{self, Mood};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let mood = match std::env::args().nth(1).as_deref() {
        Some("busy") => Mood::Busy,
        Some("sad") => Mood::Sad,
        _ => Mood::Idle,
    };
    let mut terminal = ratatui::init();
    let mut tick = 0;
    let result = loop {
        let drawn = terminal.draw(|frame| {
            let [_, area, label, _] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(mascot::HEIGHT),
                Constraint::Length(2),
                Constraint::Fill(1),
            ])
            .areas(frame.area());
            let [_, area, _] = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(mascot::WIDTH),
                Constraint::Fill(1),
            ])
            .areas(area);
            frame.render_widget(
                Paragraph::new(mascot::render(&mascot::pose(tick, mood))),
                area,
            );
            frame.render_widget(
                Paragraph::new(Line::from(format!("{mood:?}  ·  q quits"))).centered(),
                label,
            );
        });
        if let Err(error) = drawn {
            break Err(error);
        }
        match event::poll(Duration::from_millis(80)) {
            Ok(true) => {
                if let Ok(Event::Key(key)) = event::read()
                    && key.code == KeyCode::Char('q')
                {
                    break Ok(());
                }
            }
            Ok(false) => tick += 1,
            Err(error) => break Err(error),
        }
    };
    ratatui::restore();
    result
}
