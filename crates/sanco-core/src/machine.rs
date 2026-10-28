//! Which computer this is: a stable id (so a database copied to another PC
//! takes a new site id) and a display name. Windows and macOS.

use std::process::Command;

/// Stable id of this OS install: `MachineGuid` on Windows, the hardware
/// `IOPlatformUUID` on macOS. Falls back to the computer name.
pub fn id() -> String {
    platform_id()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(name)
}

/// The computer's name, as the OS shows it.
pub fn name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(mac_computer_name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "PC".into())
}

#[cfg(windows)]
fn platform_id() -> Option<String> {
    let out = run(
        "reg",
        &[
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ],
    )?;
    out.lines()
        .find(|l| l.contains("MachineGuid"))
        .and_then(|l| l.split_whitespace().last())
        .map(String::from)
}

#[cfg(target_os = "macos")]
fn platform_id() -> Option<String> {
    // `"IOPlatformUUID" = "564D…"`
    let out = run("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])?;
    out.lines()
        .find(|l| l.contains("IOPlatformUUID"))
        .and_then(|l| l.split('"').nth(3))
        .map(String::from)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_id() -> Option<String> {
    std::fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(target_os = "macos")]
fn mac_computer_name() -> Option<String> {
    run("scutil", &["--get", "ComputerName"]).map(|s| s.trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn mac_computer_name() -> Option<String> {
    None
}

/// Runs a small system tool without a console window and returns stdout.
fn run(exe: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn id_is_stable_and_not_empty() {
        let id = super::id();
        assert!(!id.is_empty());
        assert_eq!(id, super::id());
    }
}
