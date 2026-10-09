//! Desktop notifications from the background sync. SkillPouch runs on its
//! own, so what needs a person (a conflict, a sign-in that stopped working)
//! has to reach them without a command: a notification once, and the web
//! app shows it until it's dealt with.

use sp_sync::status::{Phase, SyncStatus};
use std::process::{Command, Stdio};

/// What to tell the person after a sync, if anything new: `(title, body)`.
/// Updates `status` so each thing is told once.
pub fn news(status: &mut SyncStatus, web: &str) -> Option<(String, String)> {
    if status.conflicts < status.conflicts_notified {
        status.conflicts_notified = status.conflicts;
    }
    if status.conflicts > status.conflicts_notified {
        status.conflicts_notified = status.conflicts;
        let what = crate::sync::conflict_words(status.conflicts);
        let url = status
            .pouch_id
            .as_deref()
            .map(|pouch| format!("{web}{}", crate::sync::conflicts_path(pouch)))
            .unwrap_or_else(|| web.to_string());
        return Some((
            format!("SkillPouch: {what} changed in two places"),
            format!("Every computer keeps the pouch's version until you pick one: {url}"),
        ));
    }
    let problem = match status.phase {
        Phase::Stopped | Phase::Failed => status.message.clone(),
        _ => None,
    };
    if problem.is_none() {
        status.problem_notified = None;
        return None;
    }
    if problem == status.problem_notified {
        return None;
    }
    status.problem_notified.clone_from(&problem);
    Some((
        "SkillPouch stopped syncing".into(),
        problem.unwrap_or_default(),
    ))
}

/// Shows a notification, if this desktop can. Never fails.
pub fn show(title: &str, body: &str) {
    let mut command = command(title, body);
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(target_os = "linux")]
fn command(title: &str, body: &str) -> Command {
    let mut command = Command::new("notify-send");
    command.args(["--app-name=SkillPouch", title, body]);
    command
}

#[cfg(target_os = "macos")]
fn command(title: &str, body: &str) -> Command {
    let quote = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let mut command = Command::new("osascript");
    command.args([
        "-e",
        &format!(
            "display notification {} with title {}",
            quote(body),
            quote(title)
        ),
    ]);
    command
}

#[cfg(windows)]
fn command(title: &str, body: &str) -> Command {
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
         $xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
         $text = $xml.GetElementsByTagName('text'); \
         $text.Item(0).AppendChild($xml.CreateTextNode({})) > $null; \
         $text.Item(1).AppendChild($xml.CreateTextNode({})) > $null; \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('SkillPouch').Show([Windows.UI.Notifications.ToastNotification]::new($xml))",
        quote(title),
        quote(body)
    );
    let mut command = Command::new("powershell");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &script,
    ]);
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn command(_title: &str, _body: &str) -> Command {
    Command::new("true")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> SyncStatus {
        SyncStatus {
            phase: Phase::Idle,
            pouch_id: Some("p1".into()),
            last_synced_at: None,
            message: None,
            sent: 0,
            received: 0,
            conflicts: 0,
            conflicts_notified: 0,
            problem_notified: None,
            daemon_pid: None,
            updated_at: String::new(),
        }
    }

    #[test]
    fn each_conflict_and_problem_is_told_once() {
        let mut status = status();
        assert_eq!(news(&mut status, "https://x"), None);
        status.conflicts = 2;
        let (title, body) = news(&mut status, "https://x").unwrap_or_default();
        assert_eq!(title, "SkillPouch: 2 items changed in two places");
        assert!(body.ends_with("https://x/app/p/p1/conflicts"), "{body}");
        assert_eq!(news(&mut status, "https://x"), None);
        // Resolved, then a new one: told again.
        status.conflicts = 0;
        assert_eq!(news(&mut status, "https://x"), None);
        status.conflicts = 1;
        assert!(news(&mut status, "https://x").is_some());

        status.phase = Phase::Stopped;
        status.message = Some("This computer's sign-in is no longer valid.".into());
        assert!(news(&mut status, "https://x").is_some_and(|(t, _)| t.contains("stopped")));
        assert_eq!(news(&mut status, "https://x"), None);
        status.phase = Phase::Idle;
        status.message = None;
        assert_eq!(news(&mut status, "https://x"), None);
        assert_eq!(status.problem_notified, None);
    }
}
