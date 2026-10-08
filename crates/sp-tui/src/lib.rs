//! The terminal UI: what `skillpouch` opens in an interactive terminal.
//! Every command still works without it.

mod app;
pub mod logo;
mod theme;

pub use app::{Account, Action, Connection, Launch};

use app::{App, LogoView};
use ratatui::crossterm::event::{self, Event};
use ratatui_image::FilterType;
use ratatui_image::picker::{Picker, ProtocolType};
use std::io;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// How often the screen redraws while waiting for keys (spinner speed).
const FRAME: Duration = Duration::from_millis(80);
/// The picture is scaled down to about this before the terminal gets it.
const PICTURE_SIZE: u32 = 256;

/// Runs the home screen until the user picks something to do outside it.
/// `check` starts fetching the account and returns where the answer arrives;
/// it is called again for "Refresh". `None`: this computer isn't connected.
pub fn run(
    launch: Launch,
    mut check: impl FnMut() -> Option<Receiver<Connection>>,
) -> io::Result<Action> {
    let mut terminal = ratatui::init();
    let logo = picture().unwrap_or_else(app::logo_blocks);
    let mut pending = check();
    let connection = if pending.is_some() {
        Connection::Checking
    } else {
        Connection::NotConnected
    };
    let mut app = App::new(launch, connection, logo);

    let result = loop {
        if let Some(receiver) = &pending {
            match receiver.try_recv() {
                Ok(connection) => {
                    app.set_connection(connection);
                    pending = None;
                }
                Err(TryRecvError::Disconnected) => {
                    app.set_connection(Connection::Failed("The check stopped.".into()));
                    pending = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        app.tick();
        if let Err(error) = terminal.draw(|frame| app.draw(frame)) {
            break Err(error);
        }
        match event::poll(FRAME) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => break Err(error),
        }
        let action = match event::read() {
            Ok(Event::Key(key)) => app.on_key(key),
            Ok(_) => None,
            Err(error) => break Err(error),
        };
        match action {
            Some(Action::Refresh) => {
                pending = check();
                app.set_connection(if pending.is_some() {
                    Connection::Checking
                } else {
                    Connection::NotConnected
                });
            }
            Some(action) => break Ok(action),
            None => {}
        }
    };
    ratatui::restore();
    result
}

/// The real logo, when the terminal can show pictures.
fn picture() -> Option<LogoView> {
    let picker = Picker::from_query_stdio().ok()?;
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return None;
    }
    let image = logo::image().resize(PICTURE_SIZE, PICTURE_SIZE, FilterType::Lanczos3);
    Some(LogoView::Picture(Box::new(
        picker.new_resize_protocol(image),
    )))
}
