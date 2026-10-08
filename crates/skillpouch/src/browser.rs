//! Opens a link in the user's browser; failing is fine, the link is printed too.

use std::process::{Command, Stdio};

pub fn open(url: &str) -> bool {
    candidates(url).into_iter().any(|mut command| {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    })
}

fn candidates(url: &str) -> Vec<Command> {
    let with = |program: &str, args: &[&str]| {
        let mut command = Command::new(program);
        command.args(args).arg(url);
        command
    };
    if cfg!(target_os = "macos") {
        vec![with("open", &[])]
    } else if cfg!(windows) {
        vec![with("cmd", &["/C", "start", ""])]
    } else if std::env::var_os("WSL_DISTRO_NAME").is_some() {
        vec![with("wslview", &[]), with("explorer.exe", &[])]
    } else {
        vec![with("xdg-open", &[])]
    }
}
