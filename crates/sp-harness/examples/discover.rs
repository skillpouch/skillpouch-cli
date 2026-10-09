//! Prints what `discover` finds on this computer:
//! `cargo run -q -p sp-harness --example discover`.

fn main() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let places = sp_harness::Places::of(&home, &home.join(".skillpouch"));
    for harness in sp_harness::discover(&places) {
        println!(
            "{} ({}): {}",
            harness.name,
            harness.root.display(),
            harness.summary()
        );
        for item in &harness.items {
            println!(
                "  {:?} {} ({} files, {} bytes)",
                item.kind, item.name, item.files, item.bytes
            );
        }
    }
}
