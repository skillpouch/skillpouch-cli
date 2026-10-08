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

/// The pouch mark beside the name and tagline, shaded light to dark.
pub fn banner(version: &str) {
    const MARK: [&str; 5] = [
        "   ▄▀▀▀▄   ",
        "  ▀▄▄▄▄▄▀  ",
        " ▄███████▄ ",
        " █████████ ",
        "  ▀█████▀  ",
    ];
    let [light, middle, dark] = shades(false);
    let text = [
        String::new(),
        format!("{} {}", heading("SkillPouch"), dim(version)),
        "Your skills, agents and MCP servers on every computer.".to_string(),
        dim("End-to-end encrypted.").to_string(),
        String::new(),
    ];
    println!();
    for (index, (mark, text)) in MARK.iter().zip(text).enumerate() {
        let shade = match index {
            0 | 1 => &light,
            2 | 3 => &middle,
            _ => &dark,
        };
        println!("  {}  {}", shade.apply_to(mark), text);
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
