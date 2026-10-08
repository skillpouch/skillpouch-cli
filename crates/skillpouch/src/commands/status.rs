use crate::context::{Context, signed_out};
use anyhow::Result;
use serde_json::json;
use sp_api::types::Me;
use std::process::ExitCode;

pub async fn run(ctx: &Context, as_json: bool) -> Result<ExitCode> {
    let mut session = ctx.session()?;
    let token = session.access_token(ctx, false).await?;
    let me = match ctx.client.me(&session.key, &token).await {
        // The server can end a token early (e.g. after a key rotation).
        Err(error) if error.code() == Some("token_invalid") => {
            let token = session.access_token(ctx, true).await?;
            ctx.client.me(&session.key, &token).await
        }
        result => result,
    }
    .map_err(signed_out)?;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&to_json(ctx, &me))?);
    } else {
        print_human(ctx, &me);
    }
    Ok(ExitCode::SUCCESS)
}

fn to_json(ctx: &Context, me: &Me) -> serde_json::Value {
    json!({
        "server": ctx.client.base(),
        "email": me.user.email,
        "name": me.user.name,
        "plan": { "slug": me.account.entitlements.plan_slug, "name": me.account.entitlements.plan_name },
        "storage": { "used_bytes": me.account.used_bytes, "quota_bytes": me.account.quota_bytes },
        "device": me.device.as_ref().map(|d| json!({ "id": d.id, "name": d.name })),
        "delete_after": me.account.delete_after,
    })
}

fn print_human(ctx: &Context, me: &Me) {
    println!("Logged in as {} ({})", me.user.name, me.user.email);
    if let Some(device) = &me.device {
        println!("This computer  {}", device.name);
    }
    println!("Plan           {}", me.account.entitlements.plan_name);
    println!(
        "Storage        {} of {}",
        bytes(me.account.used_bytes),
        bytes(me.account.quota_bytes)
    );
    println!("Server         {}", ctx.client.base());
    if let Some(date) = &me.account.delete_after {
        println!("\nYour account is scheduled for deletion on {date}.");
    }
}

/// `1536` → `1.5 KB`.
fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::bytes;

    #[test]
    fn formats_sizes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(1_073_741_824), "1.0 GB");
    }
}
