//! Prints this computer's token usage per day: `cargo run -p sp-usage --example scan`.

fn main() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let started = std::time::Instant::now();
    let rows = sp_usage::scan(&sp_usage::Locations::of(std::path::Path::new(&home)));
    for row in &rows {
        println!(
            "{} {:12} {:28} {:>12}",
            row.day,
            row.assistant,
            row.model,
            row.counts.total()
        );
    }
    eprintln!("{} rows in {:?}", rows.len(), started.elapsed());
}
