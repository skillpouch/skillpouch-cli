//! Everything the CLI prints for people: the banner, status lines, cards and
//! spinners. Machine output (`--json`) bypasses this module.

mod card;
mod spinner;
mod theme;

pub use card::{Card, meter};
pub use spinner::Spinner;
pub use theme::{accent, dim, heading, rich, shades};

use console::Term;

/// Columns available on stdout; 80 when it isn't a terminal.
pub fn width() -> usize {
    let (_, columns) = Term::stdout().size_checked().unwrap_or((24, 80));
    usize::from(columns)
}

/// `✔  message`. Two spaces: some terminals draw these symbols wide.
pub fn success(message: impl AsRef<str>) {
    println!("{}  {}", theme::success("✔"), rich(message.as_ref(), false));
}

/// `› message`, for what to do next.
pub fn hint(message: impl AsRef<str>) {
    println!("{}  {}", accent("›"), rich(message.as_ref(), false));
}

/// `! message` in yellow.
pub fn warn(message: impl AsRef<str>) {
    println!(
        "{}  {}",
        theme::warn("!"),
        theme::warn(rich(message.as_ref(), false))
    );
}

/// `✖ message` on stderr, with an optional `› hint` under it.
pub fn error(message: impl AsRef<str>, hint: Option<&str>) {
    eprintln!("{}  {}", theme::error("✖"), rich(message.as_ref(), true));
    if let Some(hint) = hint {
        eprintln!("   {}  {}", theme::accent_err("›"), rich(hint, true));
    }
}

/// A short command list: `name` in accent, then what it does.
pub fn commands(rows: &[(&str, &str)]) {
    let width = rows.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    for (name, about) in rows {
        println!("  {}  {}", accent(format!("{name:<width$}")), dim(about));
    }
}

/// The SkillPouch pouch, traced from the logo at two pixels per cell, beside
/// the name and tagline.
pub fn banner(version: &str) {
    const MARK: [&str; 9] = [
        "    ▄█▀█▄  ▄█▀█▄    ",
        "     ▀█▄█▀██▄█▀     ",
        "     ██████████     ",
        "   ▄████████████▄   ",
        " ▄████████████████▄ ",
        "████████████████████",
        "████████████████████",
        "▀██████████████████▀",
        "  ▀▀████████████▀▀  ",
    ];
    let purple = theme::brand();
    let mut text = vec![String::new(); MARK.len()];
    text[3] = format!("{} {}", heading("SkillPouch"), dim(version));
    text[4] = "Your skills, agents and MCP servers on every computer.".to_string();
    text[5] = dim("End-to-end encrypted.").to_string();
    println!();
    for (mark, text) in MARK.iter().zip(text) {
        println!("  {}   {}", purple.apply_to(mark), text);
    }
    println!();
}

/// A code the user compares with the browser, framed so it stands out.
pub fn code_box(code: &str) {
    let border = &shades(false)[1];
    let shown = code.replace('-', " · ");
    let inner = console::measure_text_width(&shown) + 6;
    println!("  {}", border.apply_to(format!("╭{}╮", "─".repeat(inner))));
    println!(
        "  {}   {}   {}",
        border.apply_to("│"),
        accent(&shown).bold(),
        border.apply_to("│")
    );
    println!("  {}", border.apply_to(format!("╰{}╯", "─".repeat(inner))));
}
