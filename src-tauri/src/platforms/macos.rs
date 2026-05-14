#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
struct AutomationTarget {
    label: &'static str,
    script: String,
    required: bool,
}

#[cfg(target_os = "macos")]
pub fn request_automation_permission(codex_executable: &str) -> Result<(), String> {
    let mut targets = vec![
        AutomationTarget {
            label: "System Events",
            script: r#"tell application "System Events" to get name"#.to_string(),
            required: true,
        },
        AutomationTarget {
            label: "Finder",
            script: r#"tell application "Finder" to get name"#.to_string(),
            required: true,
        },
    ];

    if let Some(computer_use_app) = computer_use_app_path(codex_executable) {
        targets.push(AutomationTarget {
            label: "Codex Computer Use",
            script: format!(
                r#"tell application "{}" to get name"#,
                applescript_string(&computer_use_app.to_string_lossy())
            ),
            required: true,
        });
    } else {
        targets.push(AutomationTarget {
            label: "Codex Computer Use",
            script: r#"tell application id "com.openai.sky.CUAService" to get name"#.to_string(),
            required: false,
        });
    }

    for target in targets {
        match request_target(&target.script) {
            Ok(()) => {}
            Err(err) if is_automation_denied(&err) => {
                return Err(automation_denied_message(target.label, &err));
            }
            Err(err) if target.required => {
                return Err(format!(
                    "failed to request macOS Automation permission for {}: {}",
                    target.label, err
                ));
            }
            Err(err) => {
                eprintln!(
                    "Skipping macOS Automation preflight for {}: {}",
                    target.label, err
                );
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn request_target(script: &str) -> Result<(), String> {
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("failed to request macOS Automation permission: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

#[cfg(target_os = "macos")]
fn computer_use_app_path(codex_executable: &str) -> Option<PathBuf> {
    let contents_dir = Path::new(codex_executable).parent()?.parent()?;
    let path = contents_dir
        .join("Resources")
        .join("plugins")
        .join("openai-bundled")
        .join("plugins")
        .join("computer-use")
        .join("Codex Computer Use.app");
    path.is_dir().then_some(path)
}

#[cfg(target_os = "macos")]
fn applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "macos")]
fn is_automation_denied(stderr: &str) -> bool {
    stderr.contains("-1743")
        || stderr
            .to_ascii_lowercase()
            .contains("not authorized to send apple events")
}

#[cfg(target_os = "macos")]
fn automation_denied_message(target: &str, stderr: &str) -> String {
    format!(
        "macOS Automation permission is denied for CodexL to control {}. Enable CodexL in System Settings > Privacy & Security > Automation, then start Codex again.{}",
        target,
        if stderr.is_empty() {
            String::new()
        } else {
            format!(" ({})", stderr)
        }
    )
}

#[cfg(not(target_os = "macos"))]
pub fn request_automation_permission(_codex_executable: &str) -> Result<(), String> {
    Ok(())
}
