//! Prints this computer's token usage per day: `cargo run -p sp-usage --example scan`.
//! `SCAN_CACHE=<file>` remembers what was read, like the CLI does.

fn main() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let started = std::time::Instant::now();
    let locations = sp_usage::Locations {
        cache: std::env::var_os("SCAN_CACHE").map(Into::into),
        ..sp_usage::Locations::of(std::path::Path::new(&home))
    };
    let rows = sp_usage::scan(&locations);
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
