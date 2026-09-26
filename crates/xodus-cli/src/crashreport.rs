//! Turning a failed launch into a report worth reading.
//!
//! A Wine log is long, noisy, and full of the player's home directory and
//! Xbox account. What leaves here is trimmed and scrubbed, and the player
//! sees it before it goes anywhere.

use std::path::{Path, PathBuf};

/// Enough log to see what happened without overflowing a GitHub URL.
const LOG_TAIL: usize = 60;
const BODY_LIMIT: usize = 1800;

pub fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{home}/.local/share/xpedited/logs"))
}

/// Keep the most recent logs and drop the rest, so this never grows without
/// bound on a machine where a lot of games are tried.
pub fn prune(keep: usize) {
    let Ok(entries) = std::fs::read_dir(log_dir()) else {
        return;
    };
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    logs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in logs.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

/// Anything that identifies the person running the game.
#[derive(Default, Clone)]
pub struct Private {
    pub home: String,
    pub user: String,
    pub gamertag: String,
    pub xuid: String,
}

impl Private {
    pub fn here(identity: Option<&(String, String)>) -> Self {
        Self {
            home: std::env::var("HOME").unwrap_or_default(),
            user: std::env::var("USER").unwrap_or_default(),
            xuid: identity.map(|(xuid, _)| xuid.clone()).unwrap_or_default(),
            gamertag: identity
                .map(|(_, gamertag)| gamertag.clone())
                .unwrap_or_default(),
        }
    }

    /// Best effort on known values, which is why the report is shown first.
    pub fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (value, placeholder) in [
            (&self.home, "~"),
            (&self.gamertag, "<gamertag>"),
            (&self.xuid, "<xuid>"),
        ] {
            if value.len() > 2 {
                out = out.replace(value.as_str(), placeholder);
            }
        }
        // The username turns up in paths Wine builds for itself, which the
        // home directory replacement does not cover.
        if self.user.len() > 2 {
            out = out.replace(&format!("/{}/", self.user), "/<user>/");
        }
        // Rewrite account lines whole; we may not know every value on them.
        out.lines()
            .map(|line| {
                if line.contains("signed in as ") {
                    return "signed in as <gamertag> (<xuid>)".to_string();
                }
                redact_secrets(line)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Replace anything on a line that looks like a credential or an address.
///
/// Logs are written by the game and by Wine, so they carry things we never
/// put there. The report is shown before sending, but it should not depend
/// on someone spotting a token in a wall of text.
fn redact_secrets(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for word in line.split_inclusive(char::is_whitespace) {
        let bare = word.trim_matches(|c: char| !c.is_ascii_graphic());
        let looks_secret = bare.len() >= 40
            && bare
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c))
            && bare.chars().any(|c| c.is_ascii_digit())
            && bare.chars().any(|c| c.is_ascii_uppercase());
        let looks_like_email = bare.contains('@') && bare.contains('.') && bare.len() > 5;
        if looks_secret {
            out.push_str("<redacted>");
            if word.ends_with(char::is_whitespace) {
                out.push(' ');
            }
        } else if looks_like_email {
            out.push_str("<email>");
            if word.ends_with(char::is_whitespace) {
                out.push(' ');
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

/// Why we think this was a failure, or None if it looks like a normal exit.
pub fn diagnose(exit_code: Option<i32>, log: &str, seconds: u64) -> Option<String> {
    const MARKERS: [(&str, &str); 7] = [
        ("No .msixvc file found", "not available in a format we can install"),
        ("Unhandled exception", "unhandled exception"),
        ("wine: Call from", "bad call into Wine"),
        ("err:seh:", "crashed"),
        ("Assertion failed", "assertion failed"),
        ("panicked at", "the launcher crashed"),
        ("Segmentation fault", "segfault"),
    ];

    for (marker, reason) in MARKERS {
        if log.contains(marker) {
            return Some(reason.to_string());
        }
    }
    match exit_code {
        Some(0) | None => None,
        Some(code) if seconds < 20 => Some(format!("closed immediately (exit {code})")),
        Some(code) => Some(format!("exited (code {code})")),
    }
}

fn first_line_of(command: &str, args: &[&str]) -> String {
    std::process::Command::new(command)
        .args(args)
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8(out.stdout)
                .ok()
                .and_then(|text| text.lines().next().map(str::to_string))
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// What the maintainer will want to know before reading the log.
pub fn environment(wine: &str) -> Vec<(String, String)> {
    let distro = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME=").map(str::to_string))
        })
        .map(|name| name.trim_matches('"').to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let graphics = std::fs::read_to_string("/proc/driver/nvidia/version")
        .ok()
        .and_then(|text| text.lines().next().map(str::to_string))
        .or_else(|| {
            // No shell here: a pipeline would mean handing a string to sh.
            let out = std::process::Command::new("lspci").output().ok()?;
            String::from_utf8(out.stdout).ok()?.lines().find_map(|line| {
                let lower = line.to_lowercase();
                (lower.contains("vga") || lower.contains("3d controller"))
                    .then(|| line.to_string())
            })
        })
        .unwrap_or_else(|| "unknown".to_string());

    vec![
        ("Xpedited".into(), env!("CARGO_PKG_VERSION").into()),
        ("Distribution".into(), distro),
        ("Kernel".into(), first_line_of("uname", &["-r"])),
        ("Wine".into(), first_line_of(wine, &["--version"])),
        ("Graphics".into(), graphics.trim().to_string()),
    ]
}

pub struct Report {
    pub title: String,
    pub body: String,
}

pub fn build(
    game: &str,
    store_id: &str,
    reason: &str,
    log_path: &Path,
    wine: &str,
    private: &Private,
) -> Report {
    let log = std::fs::read_to_string(log_path).unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    let tail = lines[lines.len().saturating_sub(LOG_TAIL)..].join("\n");
    let mut tail = private.scrub(&tail);
    if tail.len() > BODY_LIMIT {
        let cut = tail.len() - BODY_LIMIT;
        tail = format!("[earlier output trimmed]\n{}", &tail[cut..]);
    }

    let mut body = String::new();
    body.push_str(&format!("**{game}** (`{store_id}`) — {reason}.\n\n"));
    body.push_str("| | |\n|---|---|\n");
    for (key, value) in environment(wine) {
        body.push_str(&format!("| {key} | {} |\n", private.scrub(&value)));
    }
    body.push_str("\n<details><summary>Log</summary>\n\n```\n");
    body.push_str(&tail);
    body.push_str("\n```\n\n</details>\n\n");
    body.push_str("_Paths, gamertag and XUID are redacted._\n");

    Report {
        title: format!("{game}: {reason}"),
        body,
    }
}

/// Percent-encode for a query string. Everything that is not unreserved.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Whether a setting is a plausible `owner/name`, so a stray value cannot
/// send a report somewhere unexpected.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let sane = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    sane(owner) && sane(name)
}

pub fn issue_url(repo: &str, report: &Report) -> String {
    format!(
        "https://github.com/{repo}/issues/new?labels=game-report&title={}&body={}",
        encode(&report.title),
        encode(&report.body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A title published only as UWP is not a broken download, and saying
    /// so saves the player retrying something that cannot work.
    #[test]
    fn an_unsupported_package_is_explained_not_blamed() {
        let reason = diagnose(Some(1), "No .msixvc file found\n", 4).unwrap();
        assert!(reason.contains("install"));
        assert!(!reason.contains("exit code"));
    }

    #[test]
    fn a_clean_exit_is_not_a_crash() {
        assert!(diagnose(Some(0), "all fine", 600).is_none());
        assert!(diagnose(None, "still running", 600).is_none());
    }

    #[test]
    fn failures_are_recognised_and_explained() {
        assert!(diagnose(Some(1), "", 3).unwrap().contains("immediately"));
        assert!(diagnose(Some(1), "", 600).unwrap().contains("code 1"));
        // A marker beats the exit code, because it says more.
        let reason = diagnose(Some(0), "err:seh:call_stack_handlers", 600).unwrap();
        assert!(reason.contains("crashed"));
    }

    /// The account line is rewritten whole, because the values on it come
    /// from the launcher's own lookup and need not match ours.
    #[test]
    fn the_account_line_is_scrubbed_even_when_we_guessed_wrong() {
        let private = Private {
            home: "/home/someone".into(),
            user: "someone".into(),
            gamertag: "Stale".into(),
            xuid: "1111111111111111".into(),
        };
        let scrubbed =
            private.scrub("starting\nsigned in as RealTag (2533274800000000)\nloading");
        assert!(!scrubbed.contains("2533274800000000"));
        assert!(!scrubbed.contains("RealTag"));
        assert!(scrubbed.contains("signed in as <gamertag> (<xuid>)"));
        assert!(scrubbed.contains("starting") && scrubbed.contains("loading"));
    }

    /// A log can carry a token or an address we never put there.
    #[test]
    fn credentials_in_the_log_are_redacted() {
        let private = Private::default();
        let token = "eyJhbGciOiJSUzI1NiIsImtpZCI6IkFBQUJCQkNDQ0RERA.eyJzdWIiOiIxMjM0";
        let out = private.scrub(&format!("Authorization: Bearer {token}\nmail reed@example.com"));
        assert!(!out.contains(token), "token survived: {out}");
        assert!(!out.contains("reed@example.com"), "email survived: {out}");
        assert!(out.contains("Authorization:"), "context should remain: {out}");
    }

    /// Ordinary log lines must not be mangled by the redaction.
    #[test]
    fn normal_lines_are_left_alone() {
        let private = Private::default();
        let line = "fixme:d3d:wined3d_swapchain_present Unimplemented flags 0x20c.";
        assert_eq!(private.scrub(line), line);
    }

    #[test]
    fn personal_details_do_not_survive() {
        let private = Private {
            home: "/home/someone".into(),
            user: "someone".into(),
            gamertag: "CoolGamer99".into(),
            xuid: "2535000000000000".into(),
        };
        let scrubbed = private.scrub(
            "loading /home/someone/Games/x\n\
             signed in as CoolGamer99 (2535000000000000)\n\
             prefix /var/run/someone/wine",
        );
        assert!(!scrubbed.contains("/home/someone"));
        assert!(!scrubbed.contains("CoolGamer99"));
        assert!(!scrubbed.contains("2535000000000000"));
        assert!(!scrubbed.contains("/someone/"));
        assert!(scrubbed.contains("~/Games/x"));
    }

    /// GitHub refuses a URL much past 8 kB, and a Wine log is enormous, so
    /// the body has to stay small however much output the game produced.
    #[test]
    fn a_huge_log_still_makes_a_usable_url() {
        let noisy: String = (0..4000)
            .map(|i| format!("fixme:d3d:wined3d_something {i} & <unknown> [quoted]\n"))
            .collect();
        let dir = std::env::temp_dir().join("xpedited-report-test");
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("huge.log");
        std::fs::write(&log, &noisy).unwrap();

        let report = build(
            "Some Game",
            "9ABCDEFGHIJK",
            "it exited with code 1",
            &log,
            "/bin/true",
            &Private::default(),
        );
        let url = issue_url("owner/name", &report);
        assert!(
            url.len() < 8000,
            "url was {} characters, GitHub will reject it",
            url.len()
        );
        assert!(report.body.contains("earlier output trimmed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_plain_owner_name_is_accepted() {
        assert!(valid_repo("Reedo22/xpedited"));
        assert!(valid_repo("a/b"));
        for bad in [
            "", "/", "owner", "owner/name/extra", "owner/", "/name",
            "owner/name?x=1", "owner/../etc", "own er/name", "owner/na me",
            "owner/name#frag", "javascript:alert(1)/x",
        ] {
            assert!(!valid_repo(bad), "{bad:?} should have been refused");
        }
    }

    #[test]
    fn the_url_survives_a_body_full_of_punctuation() {
        let report = Report {
            title: "Game: exited (code 1)".into(),
            body: "err:seh: a & b = c?\n```\n".into(),
        };
        let url = issue_url("owner/repo", &report);
        assert!(url.starts_with("https://github.com/owner/repo/issues/new?"));
        assert!(!url.contains('\n'));
        assert!(!url.contains(' '));
        assert!(url.contains("%26")); // the ampersand is encoded, not a separator
    }
}

