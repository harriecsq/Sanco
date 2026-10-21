//! Tailscale address checks and detection of this PC's Tailscale IP.

use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;

/// `100.64.0.0/10` or `fd7a:115c:a1e0::/48`.
pub fn is_tailscale(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (o[1] & 0xC0) == 64
        }
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// Who may talk to the sync server: Tailscale peers, and this PC itself
/// (several instances on one PC during development).
pub fn is_allowed_peer(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback() || is_tailscale(ip)
}

/// This PC's Tailscale IPv4 address, from the `tailscale` CLI. `None` if
/// Tailscale isn't installed, running or logged in. Blocks briefly.
pub fn detect_ip() -> Option<String> {
    const CANDIDATES: &[&str] = &["tailscale", r"C:\Program Files\Tailscale\tailscale.exe"];
    for exe in CANDIDATES {
        let mut cmd = Command::new(exe);
        cmd.args(["ip", "-4"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let Ok(out) = cmd.output() else { continue };
        if !out.status.success() {
            continue;
        }
        let found = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<Ipv4Addr>().ok())
            .find(|ip| is_tailscale(IpAddr::V4(*ip)));
        if let Some(ip) = found {
            return Some(ip.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(is_tailscale(ip("100.64.0.1")));
        assert!(is_tailscale(ip("100.127.255.254")));
        assert!(!is_tailscale(ip("100.128.0.1")));
        assert!(!is_tailscale(ip("100.63.255.255")));
        assert!(!is_tailscale(ip("192.168.1.10")));
        assert!(is_tailscale(ip("fd7a:115c:a1e0::1")));
        assert!(!is_tailscale(ip("fd7a:115c:a1e1::1")));
        assert!(is_tailscale(ip("::ffff:100.100.1.1")));

        assert!(is_allowed_peer(ip("127.0.0.1")));
        assert!(is_allowed_peer(ip("::1")));
        assert!(!is_allowed_peer(ip("10.0.0.5")));
        assert!(!is_allowed_peer(ip("0.0.0.0")));
    }
}
