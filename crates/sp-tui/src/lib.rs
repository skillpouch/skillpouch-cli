//! The terminal UI: what `skillpouch` opens in an interactive terminal. A
//! dashboard of the account, its pouches, token usage and computers; logging
//! in and out, and setting up what this computer syncs, happen inside it.
//! Every command still works without it.
//!
//! The TUI doesn't talk to the server itself: it sends [`Request`]s and draws
//! the [`Update`]s it gets back.

mod app;
pub mod logo;
pub mod mascot;
mod model;
mod setup;
mod theme;
pub mod usage;
mod view;

pub use model::{
    Account, Computer, Dashboard, FirstSync, Launch, LoginStep, Notice, Pouch, Request,
    SetupChoice, SyncTone, SyncView, Tone, Update,
};

use app::{App, Outcome, Status};
use ratatui::crossterm::event::{self, Event};
use std::io;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// How often the screen redraws while waiting for keys (spinner speed).
const FRAME: Duration = Duration::from_millis(80);
/// The dashboard refreshes itself this often.
const REFRESH_EVERY: Duration = Duration::from_secs(60);

/// Runs the dashboard until the user quits. `signed_in`: this computer has
/// keys, so the first [`Update`] will be the dashboard (or why it failed).
pub fn run(
    launch: Launch,
    signed_in: bool,
    mut send: impl FnMut(Request),
    updates: Receiver<Update>,
) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new(launch, signed_in);
    let mut refreshed = Instant::now();

    let result = loop {
        loop {
            match updates.try_recv() {
                Ok(update) => {
                    if matches!(update, Update::Loaded(_)) {
                        refreshed = Instant::now();
                    }
                    app.apply(update);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    app.apply(Update::Failed(
                        "The connection to the server stopped.".into(),
                    ));
                    break;
                }
            }
        }
        if matches!(app.status, Status::Ready(_))
            && app.login.is_none()
            && app.setup.is_none()
            && !app.confirm_logout
            && !app.refreshing
            && refreshed.elapsed() >= REFRESH_EVERY
        {
            refreshed = Instant::now();
            app.refreshing = true;
            send(Request::Refresh);
        }
        app.tick();
        if let Err(error) = terminal.draw(|frame| view::draw(&app, frame)) {
            break Err(error);
        }
        match event::poll(FRAME) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => break Err(error),
        }
        let outcome = match event::read() {
            Ok(Event::Key(key)) => app.on_key(key),
            Ok(_) => Outcome::Nothing,
            Err(error) => break Err(error),
        };
        match outcome {
            Outcome::Nothing => {}
            Outcome::Send(request) => send(request),
            Outcome::Quit => break Ok(()),
        }
    };
    ratatui::restore();
    result
}
