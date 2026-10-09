//! Setting up a computer after it logs in: which pouch it syncs, and how
//! the first sync treats what is already in the pouch and on this computer.
//!
//! An empty pouch starts from one assistant's setup (or from nothing). A
//! pouch with items asks first: take the pouch as it is, merge both, or
//! replace the pouch with this computer's setup. Then: which assistants and
//! which kinds this computer syncs. After that everything runs on its own.

use crate::model::{Dashboard, FirstSync, Pouch, Request, SetupChoice};
use ratatui::crossterm::event::KeyCode;
use sp_harness::{Harness, POUCH_KINDS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Pouch,
    Mode,
    Assistant,
    /// Which assistants and kinds sync here.
    Choose,
    Confirm,
}

/// What to do with a pouch that already has items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    UsePouch,
    Merge,
    UseComputer,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Self::UsePouch, Self::Merge, Self::UseComputer];

    pub fn label(self) -> &'static str {
        match self {
            Self::UsePouch => "Use the pouch",
            Self::Merge => "Merge both",
            Self::UseComputer => "Use this computer",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::UsePouch => {
                "This computer gets the pouch's items. Its own copies are backed up, then replaced."
            }
            Self::Merge => {
                "The pouch keeps its items and gets this computer's too. Same name, different content: both are kept."
            }
            Self::UseComputer => {
                "The pouch is replaced with what's on this computer, for every computer that syncs it."
            }
        }
    }

    fn needs_assistant(self) -> bool {
        !matches!(self, Self::UsePouch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Send(Request),
    /// Leave setup without saving.
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    /// The assistants on this computer, the fullest first.
    pub assistants: Vec<Harness>,
    pub step: Step,
    pub cursor: usize,
    pub pouch: Option<String>,
    pub mode: Option<Mode>,
    /// Index into `assistants`; `None` with an empty pouch starts empty.
    pub assistant: Option<usize>,
    /// Ids of the assistants turned off.
    pub skip_assistants: std::collections::BTreeSet<String>,
    /// Kinds turned off.
    pub skip_kinds: std::collections::BTreeSet<String>,
    pub saving: bool,
    pub error: Option<String>,
}

impl Setup {
    pub fn new(mut assistants: Vec<Harness>, dashboard: &Dashboard) -> Self {
        assistants.sort_by_key(|harness| std::cmp::Reverse(harness.items.len()));
        // Start on the pouch this computer already syncs, if any.
        let cursor = dashboard
            .active_pouch()
            .and_then(|active| pouches(dashboard).iter().position(|p| p.id == active.id))
            .unwrap_or(0);
        Self {
            assistants,
            step: Step::Pouch,
            cursor,
            pouch: None,
            mode: None,
            assistant: None,
            skip_assistants: Default::default(),
            skip_kinds: Default::default(),
            saving: false,
            error: None,
        }
    }

    /// Rows on the current step.
    pub fn rows(&self, dashboard: &Dashboard) -> usize {
        match self.step {
            Step::Pouch => pouches(dashboard).len(),
            Step::Mode => Mode::ALL.len(),
            // An empty pouch can also start with nothing.
            Step::Assistant => self.assistants.len() + usize::from(self.mode.is_none()),
            // Assistants, kinds, then "Continue".
            Step::Choose => self.assistants.len() + POUCH_KINDS.len() + 1,
            Step::Confirm => 0,
        }
    }

    pub fn chosen_pouch<'a>(&self, dashboard: &'a Dashboard) -> Option<&'a Pouch> {
        dashboard.pouch(self.pouch.as_deref()?)
    }

    pub fn chosen_assistant(&self) -> Option<&Harness> {
        self.assistants.get(self.assistant?)
    }

    /// The assistant under the cursor on the assistant step.
    pub fn highlighted(&self) -> Option<&Harness> {
        (self.step == Step::Assistant)
            .then(|| self.assistants.get(self.cursor))
            .flatten()
    }

    /// What was chosen, once there is enough to save.
    pub fn choice(&self) -> Option<SetupChoice> {
        let assistant = || self.chosen_assistant().map(|h| h.id.to_string());
        let first_sync = match self.mode {
            None => FirstSync::Fill(assistant()),
            Some(Mode::UsePouch) => FirstSync::UsePouch,
            Some(Mode::Merge) => FirstSync::Merge(assistant()?),
            Some(Mode::UseComputer) => FirstSync::UseComputer(assistant()?),
        };
        Some(SetupChoice {
            pouch_id: self.pouch.clone()?,
            first_sync,
            skip_assistants: self.skip_assistants.iter().cloned().collect(),
            skip_kinds: self.skip_kinds.iter().cloned().collect(),
        })
    }

    pub fn on_key(&mut self, code: KeyCode, dashboard: &Dashboard) -> Outcome {
        if self.saving {
            return Outcome::Stay;
        }
        self.error = None;
        let rows = self.rows(dashboard);
        match code {
            KeyCode::Down | KeyCode::Char('j') if rows > 0 => {
                self.cursor = (self.cursor + 1) % rows;
                Outcome::Stay
            }
            KeyCode::Up | KeyCode::Char('k') if rows > 0 => {
                self.cursor = self.cursor.checked_sub(1).unwrap_or(rows - 1);
                Outcome::Stay
            }
            KeyCode::Char(' ') if self.step == Step::Choose => {
                self.toggle();
                Outcome::Stay
            }
            KeyCode::Enter => self.choose(dashboard),
            KeyCode::Esc | KeyCode::Backspace => self.back(dashboard),
            _ => Outcome::Stay,
        }
    }

    fn choose(&mut self, dashboard: &Dashboard) -> Outcome {
        match self.step {
            Step::Pouch => {
                let Some(pouch) = pouches(dashboard).get(self.cursor).copied() else {
                    return Outcome::Stay;
                };
                self.pouch = Some(pouch.id.clone());
                self.mode = None;
                self.assistant = None;
                if pouch.items == 0 && !pouch.more {
                    self.go(Step::Assistant, 0);
                } else {
                    self.go(Step::Mode, 0);
                }
            }
            Step::Mode => {
                let mode = Mode::ALL[self.cursor.min(Mode::ALL.len() - 1)];
                if mode.needs_assistant() && self.assistants.is_empty() {
                    self.error =
                        Some("No assistant found on this computer to take items from.".into());
                    return Outcome::Stay;
                }
                self.mode = Some(mode);
                self.assistant = None;
                if mode.needs_assistant() {
                    self.go(Step::Assistant, 0);
                } else {
                    self.go(Step::Choose, self.continue_row());
                }
            }
            Step::Assistant => {
                // The row after the assistants is "start empty".
                self.assistant = (self.cursor < self.assistants.len()).then_some(self.cursor);
                // What the first sync starts from stays on.
                if let Some(harness) = self.chosen_assistant() {
                    let id = harness.id.to_string();
                    self.skip_assistants.remove(&id);
                }
                self.go(Step::Choose, self.continue_row());
            }
            Step::Choose if self.cursor < self.continue_row() => self.toggle(),
            Step::Choose => self.go(Step::Confirm, 0),
            Step::Confirm => {
                let Some(choice) = self.choice() else {
                    return Outcome::Stay;
                };
                self.saving = true;
                return Outcome::Send(Request::FinishSetup(choice));
            }
        }
        Outcome::Stay
    }

    fn back(&mut self, dashboard: &Dashboard) -> Outcome {
        let mode_row = |mode: Option<Mode>| {
            mode.and_then(|mode| Mode::ALL.iter().position(|m| *m == mode))
                .unwrap_or(0)
        };
        match self.step {
            Step::Pouch => return Outcome::Close,
            Step::Mode => {
                let row = self.pouch_row(dashboard);
                self.go(Step::Pouch, row);
            }
            Step::Assistant if self.mode.is_some() => self.go(Step::Mode, mode_row(self.mode)),
            Step::Assistant => {
                let row = self.pouch_row(dashboard);
                self.go(Step::Pouch, row);
            }
            Step::Choose => match self.mode {
                Some(mode) if !mode.needs_assistant() => self.go(Step::Mode, mode_row(self.mode)),
                _ => {
                    let row = self.assistant.unwrap_or(self.assistants.len());
                    self.go(Step::Assistant, row);
                }
            },
            Step::Confirm => self.go(Step::Choose, self.continue_row()),
        }
        Outcome::Stay
    }

    /// The "Continue" row of the choose step.
    pub fn continue_row(&self) -> usize {
        self.assistants.len() + POUCH_KINDS.len()
    }

    /// Turns the assistant or kind under the cursor on or off.
    fn toggle(&mut self) {
        let cursor = self.cursor;
        if let Some(harness) = self.assistants.get(cursor) {
            let id = harness.id.to_string();
            if self
                .chosen_assistant()
                .is_some_and(|chosen| chosen.id == id)
            {
                self.error = Some(format!(
                    "{} is what the first sync starts from, so it stays on.",
                    harness.name
                ));
            } else if !self.skip_assistants.remove(&id) {
                self.skip_assistants.insert(id);
            }
            return;
        }
        let Some((kind, _)) = POUCH_KINDS.get(cursor - self.assistants.len()) else {
            return;
        };
        let kind = kind.to_string();
        if self.skip_kinds.remove(&kind) {
            return;
        }
        if self.skip_kinds.len() + 1 == POUCH_KINDS.len() {
            self.error = Some("Leave at least one kind on.".into());
        } else {
            self.skip_kinds.insert(kind);
        }
    }

    fn pouch_row(&self, dashboard: &Dashboard) -> usize {
        pouches(dashboard)
            .iter()
            .position(|p| Some(&p.id) == self.pouch.as_ref())
            .unwrap_or(0)
    }

    fn go(&mut self, step: Step, cursor: usize) {
        self.step = step;
        self.cursor = cursor;
    }
}

/// The pouches a computer can sync.
pub fn pouches(dashboard: &Dashboard) -> Vec<&Pouch> {
    dashboard.pouches.iter().filter(|p| p.usable()).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::tests::dashboard;
    use sp_harness::{Item, Kind};
    use std::path::PathBuf;

    pub fn assistants() -> Vec<Harness> {
        let item = |kind, name: &str| Item {
            kind,
            name: name.into(),
            path: PathBuf::from(name),
            files: 1,
            bytes: 10,
        };
        vec![
            Harness {
                id: "codex",
                name: "Codex",
                root: PathBuf::from("/home/ada/.codex"),
                items: vec![item(Kind::Instructions, "AGENTS.md")],
            },
            Harness {
                id: "claude-code",
                name: "Claude Code",
                root: PathBuf::from("/home/ada/.claude"),
                items: vec![
                    item(Kind::Instructions, "CLAUDE.md"),
                    item(Kind::Skill, "pdf-tools"),
                    item(Kind::Skill, "reviewer"),
                    item(Kind::McpServer, "github"),
                ],
            },
        ]
    }

    /// The fixture with its second pouch usable and empty.
    pub fn board() -> Dashboard {
        let mut board = dashboard();
        board.pouches[1].state = "active".into();
        board.pouches[1].items = 0;
        board
    }

    fn keys(setup: &mut Setup, board: &Dashboard, codes: &[KeyCode]) -> Outcome {
        codes
            .iter()
            .map(|code| setup.on_key(*code, board))
            .last()
            .unwrap_or(Outcome::Stay)
    }

    #[test]
    fn an_empty_pouch_starts_from_the_fullest_assistant() {
        let board = board();
        let mut setup = Setup::new(assistants(), &board);
        // Starts on the pouch this computer already syncs.
        assert_eq!((setup.step, setup.cursor), (Step::Pouch, 0));
        assert_eq!(setup.assistants[0].id, "claude-code");
        keys(&mut setup, &board, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(setup.step, Step::Assistant);
        assert_eq!(setup.rows(&board), 3);
        assert_eq!(setup.highlighted().map(|h| h.id), Some("claude-code"));
        keys(&mut setup, &board, &[KeyCode::Enter]);
        assert_eq!(setup.step, Step::Choose);
        // Codex off, hooks off; Claude Code feeds the first sync, so it stays.
        setup.cursor = 1;
        keys(&mut setup, &board, &[KeyCode::Char(' ')]);
        setup.cursor = 0;
        keys(&mut setup, &board, &[KeyCode::Char(' ')]);
        assert!(
            setup
                .error
                .as_deref()
                .is_some_and(|e| e.contains("stays on"))
        );
        setup.cursor = 2 + 5;
        keys(&mut setup, &board, &[KeyCode::Enter]);
        setup.cursor = setup.continue_row();
        keys(&mut setup, &board, &[KeyCode::Enter]);
        assert_eq!(setup.step, Step::Confirm);
        assert_eq!(
            keys(&mut setup, &board, &[KeyCode::Enter]),
            Outcome::Send(Request::FinishSetup(SetupChoice {
                pouch_id: board.pouches[1].id.clone(),
                first_sync: FirstSync::Fill(Some("claude-code".into())),
                skip_assistants: vec!["codex".into()],
                skip_kinds: vec!["hook".into()],
            }))
        );
        // Waiting for the save: keys do nothing.
        assert_eq!(keys(&mut setup, &board, &[KeyCode::Esc]), Outcome::Stay);
    }

    #[test]
    fn an_empty_pouch_can_start_empty() {
        let board = board();
        let mut setup = Setup::new(assistants(), &board);
        keys(
            &mut setup,
            &board,
            &[KeyCode::Down, KeyCode::Enter, KeyCode::Up, KeyCode::Enter],
        );
        assert_eq!(
            setup.choice().map(|c| c.first_sync),
            Some(FirstSync::Fill(None))
        );
    }

    #[test]
    fn a_pouch_with_items_asks_how_to_start() {
        let board = board();
        let mut setup = Setup::new(assistants(), &board);
        keys(&mut setup, &board, &[KeyCode::Enter]);
        assert_eq!(setup.step, Step::Mode);
        // Use the pouch: nothing to pick from this computer.
        keys(&mut setup, &board, &[KeyCode::Enter]);
        assert_eq!(setup.step, Step::Choose);
        assert_eq!(setup.cursor, setup.continue_row());
        assert_eq!(
            setup.choice().map(|c| c.first_sync),
            Some(FirstSync::UsePouch)
        );
        // Back to the modes, then merge with Codex.
        keys(&mut setup, &board, &[KeyCode::Esc]);
        assert_eq!((setup.step, setup.cursor), (Step::Mode, 0));
        keys(
            &mut setup,
            &board,
            &[KeyCode::Down, KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
        );
        assert_eq!(setup.step, Step::Choose);
        assert_eq!(
            setup.choice().map(|c| c.first_sync),
            Some(FirstSync::Merge("codex".into()))
        );
        // Back lands on the chosen assistant, then on the chosen mode.
        keys(&mut setup, &board, &[KeyCode::Esc]);
        assert_eq!((setup.step, setup.cursor), (Step::Assistant, 1));
        keys(&mut setup, &board, &[KeyCode::Esc]);
        assert_eq!((setup.step, setup.cursor), (Step::Mode, 1));
        keys(&mut setup, &board, &[KeyCode::Esc]);
        assert_eq!(setup.step, Step::Pouch);
        assert_eq!(keys(&mut setup, &board, &[KeyCode::Esc]), Outcome::Close);
    }

    #[test]
    fn locked_pouches_are_not_offered_and_nothing_to_take_says_so() {
        let board = dashboard();
        assert_eq!(pouches(&board).len(), 1);
        let mut setup = Setup::new(Vec::new(), &board);
        keys(
            &mut setup,
            &board,
            &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
        );
        assert_eq!(setup.step, Step::Mode);
        assert!(
            setup
                .error
                .as_deref()
                .is_some_and(|e| e.contains("No assistant"))
        );
    }
}
