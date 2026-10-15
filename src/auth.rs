use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use thiserror::Error;
use wait_timeout::ChildExt;

/// A locked keyring or stalled CLI must not block server startup indefinitely.
const GH_AUTH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("no GitHub token found: set GH_TOKEN, GITHUB_TOKEN, or authenticate gh CLI")]
    NoToken,
    #[error("no stored GitHub token found for the selected account; authenticate that account with gh CLI")]
    SelectedAccountMissing,
    #[error("invalid GitHub account selector; use a GitHub login")]
    InvalidAccount,
    #[error("gh CLI auth failed: {0}")]
    GhCli(String),
    #[error("gh CLI auth timed out: {0}")]
    Timeout(String),
}

/// Resolve once without validation requests or changes to gh's active account.
/// Only the explicit process selector may override ambient credentials.
pub fn resolve_token() -> Result<String, AuthError> {
    let account = selected_account()?;
    resolve_token_from_sources(
        account.as_deref(),
        std::env::var("GH_TOKEN").ok().as_deref(),
        std::env::var("GITHUB_TOKEN").ok().as_deref(),
        stored_token,
    )
}

pub fn selected_account() -> Result<Option<String>, AuthError> {
    match std::env::var("KP_GITHUB_ACCOUNT") {
        Ok(account) => {
            validate_account(&account)?;
            Ok(Some(account))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(AuthError::InvalidAccount),
    }
}

pub fn validate_account(account: &str) -> Result<(), AuthError> {
    if account.is_empty()
        || account.len() > 39
        || !account
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || account.starts_with('-')
        || account.ends_with('-')
    {
        return Err(AuthError::InvalidAccount);
    }
    Ok(())
}

/// Named requests always use that stored credential, never an ambient token.
pub fn named_token(account: &str) -> Result<String, AuthError> {
    validate_account(account)?;
    stored_token(Some(account))
}

fn resolve_token_from_sources(
    account: Option<&str>,
    gh_token: Option<&str>,
    github_token: Option<&str>,
    lookup: impl FnOnce(Option<&str>) -> Result<String, AuthError>,
) -> Result<String, AuthError> {
    if let Some(account) = account {
        validate_account(account)?;
        return lookup(Some(account));
    }
    if let Some(token) = gh_token.filter(|s| !s.is_empty()) {
        return Ok(token.to_owned());
    }
    if let Some(token) = github_token.filter(|s| !s.is_empty()) {
        return Ok(token.to_owned());
    }
    lookup(None)
}

fn stored_token(account: Option<&str>) -> Result<String, AuthError> {
    let mut cmd = Command::new("gh");
    configure_stored_token_command(&mut cmd, account);
    let (success, stdout) = run_with_timeout(cmd, GH_AUTH_TIMEOUT)?;
    if success && !stdout.trim().is_empty() {
        return Ok(stdout.trim().to_owned());
    }
    Err(if account.is_some() {
        AuthError::SelectedAccountMissing
    } else {
        AuthError::NoToken
    })
}

fn configure_stored_token_command(cmd: &mut Command, account: Option<&str>) {
    cmd.args(["auth", "token", "--hostname", "github.com"]);
    if let Some(account) = account {
        cmd.args(["--user", account]);
    }
    // gh auth token otherwise prefers environment tokens, including when a
    // stored account was requested. Remove only this child's conflicting vars.
    cmd.env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_HOST")
        .env_remove("GH_DEBUG")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Never surface CLI auth output that might contain a credential.
        .stderr(Stdio::null());
}

/// Spawn `cmd`, wait up to `timeout`, kill on timeout. `gh auth token` emits only
/// one short token, so its piped stdout cannot fill the pipe under normal use.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<(bool, String), AuthError> {
    let mut child: Child = cmd.spawn().map_err(|e| AuthError::GhCli(e.to_string()))?;
    let status = match child.wait_timeout(timeout) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AuthError::GhCli(error.to_string()));
        }
    };
    match status {
        Some(status) => {
            let mut stdout = String::new();
            if let Some(mut s) = child.stdout.take() {
                s.read_to_string(&mut stdout)
                    .map_err(|e| AuthError::GhCli(e.to_string()))?;
            }
            Ok((status.success(), stdout))
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Err(AuthError::Timeout(format!(
                "command timed out after {timeout:?}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_token_precedence_matches_gh() {
        let token =
            resolve_token_from_sources(None, Some("gh-first"), Some("github-second"), |_| {
                panic!("must use token")
            });
        assert_eq!(token.unwrap(), "gh-first");
        let token = resolve_token_from_sources(None, Some(""), Some("github-second"), |_| {
            panic!("must use token")
        });
        assert_eq!(token.unwrap(), "github-second");
    }

    #[test]
    fn empty_tokens_resolve_stored_default() {
        let token = resolve_token_from_sources(None, Some(""), Some(""), |account| {
            assert_eq!(account, None);
            Ok("stored-default".into())
        });
        assert_eq!(token.unwrap(), "stored-default");
    }

    #[test]
    fn explicit_account_wins_over_ambient_tokens() {
        let token = resolve_token_from_sources(
            Some("selected-dev"),
            Some("ambient-gh"),
            Some("ambient-github"),
            |account| {
                assert_eq!(account, Some("selected-dev"));
                Ok("selected-token".into())
            },
        );
        assert_eq!(token.unwrap(), "selected-token");
    }

    #[test]
    fn missing_selected_account_does_not_fall_back() {
        let token = resolve_token_from_sources(
            Some("missing-account"),
            Some("ambient-gh"),
            Some("ambient-github"),
            |_| Err(AuthError::SelectedAccountMissing),
        );
        assert!(matches!(token, Err(AuthError::SelectedAccountMissing)));
    }

    #[test]
    fn invalid_selector_fails_without_looking_up_or_exposing_input() {
        for account in [
            "",
            " ",
            "--hostname=other",
            "user name",
            "ghp_synthetic_secret",
            "user/",
        ] {
            let error = resolve_token_from_sources(Some(account), Some("ambient"), None, |_| {
                panic!("invalid selector must fail before lookup")
            })
            .unwrap_err();
            assert!(matches!(error, AuthError::InvalidAccount));
            assert!(account.trim().is_empty() || !error.to_string().contains(account));
        }
    }

    #[test]
    fn stored_lookup_scrubs_conflicts_and_selects_only_requested_account() {
        // Run a synthetic child with no inherited environment or real gh config.
        // A shell validates the exact arguments and environment without printing
        // the environment or fetching any real credentials.
        let mut cmd = Command::new("/bin/sh");
        cmd.env_clear();
        cmd.args([
            "-c",
            r#"
            test "$*" = 'auth token --hostname github.com --user selected-dev' || exit 11
            test -z "${GH_TOKEN+x}${GITHUB_TOKEN+x}${GH_HOST+x}${GH_DEBUG+x}" || exit 12
            test "$GH_PROMPT_DISABLED" = 1 || exit 13
            printf 'synthetic-selected-token\n'
        "#,
            "fake-gh",
        ]);
        cmd.env("GH_TOKEN", "wrong-gh")
            .env("GITHUB_TOKEN", "wrong-github")
            .env("GH_HOST", "wrong-host")
            .env("GH_DEBUG", "api");
        configure_stored_token_command(&mut cmd, Some("selected-dev"));
        let (success, stdout) = run_with_timeout(cmd, Duration::from_secs(5)).unwrap();
        assert!(success);
        assert_eq!(stdout.trim(), "synthetic-selected-token");
    }

    #[test]
    fn test_auth_error_display() {
        assert!(AuthError::NoToken.to_string().contains("no GitHub token"));
        assert!(AuthError::GhCli("test error".into())
            .to_string()
            .contains("test error"));
        assert!(AuthError::Timeout("timed out".into())
            .to_string()
            .contains("timed out"));
    }

    #[test]
    fn test_run_with_timeout_kills_hanging_child() {
        let mut cmd = Command::new("sleep");
        cmd.env_clear()
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let start = std::time::Instant::now();
        let result = run_with_timeout(cmd, Duration::from_millis(200));
        assert!(matches!(result, Err(AuthError::Timeout(_))));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn test_run_with_timeout_returns_stdout_on_success() {
        let mut cmd = Command::new("printf");
        cmd.env_clear()
            .arg("hello")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let (success, stdout) = run_with_timeout(cmd, Duration::from_secs(5)).unwrap();
        assert!(success);
        assert_eq!(stdout, "hello");
    }

    #[test]
    fn test_run_with_timeout_reports_nonzero_exit() {
        let mut cmd = Command::new("/bin/sh");
        cmd.env_clear()
            .args(["-c", "exit 1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let (success, _) = run_with_timeout(cmd, Duration::from_secs(5)).unwrap();
        assert!(!success);
    }
}
