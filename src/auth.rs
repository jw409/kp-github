use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use thiserror::Error;
use wait_timeout::ChildExt;

/// Wall-clock timeout for `gh auth token`. If the gh CLI hangs (locked keyring,
/// interactive prompt, slow shell init) we must not block server startup.
const GH_AUTH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("no GitHub token found: set GITHUB_TOKEN, GH_TOKEN, or install gh CLI")]
    NoToken,
    #[error("gh CLI auth failed: {0}")]
    GhCli(String),
    #[error("gh CLI auth timed out: {0}")]
    Timeout(String),
}

/// Resolve a GitHub token via cascade: GITHUB_TOKEN -> GH_TOKEN -> `gh auth token`
pub fn resolve_token() -> Result<String, AuthError> {
    if let Ok(t) = std::env::var("GITHUB_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    if let Ok(t) = std::env::var("GH_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }

    let mut cmd = Command::new("gh");
    cmd.args(["auth", "token"]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let (success, stdout) = run_with_timeout(cmd, GH_AUTH_TIMEOUT)?;
    if success {
        let token = stdout.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    Err(AuthError::NoToken)
}

/// Spawn `cmd`, wait up to `timeout`, kill on timeout. Returns `(success, stdout)`.
///
/// Note: stdout/stderr are piped, so children that emit more than the pipe
/// buffer (~64 KiB) without being drained could block. `gh auth token` emits
/// only a token, so this is safe here.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<(bool, String), AuthError> {
    let mut child: Child = cmd.spawn().map_err(|e| AuthError::GhCli(e.to_string()))?;

    match child
        .wait_timeout(timeout)
        .map_err(|e| AuthError::GhCli(e.to_string()))?
    {
        Some(status) => {
            let mut stdout = String::new();
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut stdout);
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
    use std::sync::Mutex;

    // Env var tests are not thread-safe — serialize them with a mutex.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_token_env() {
        std::env::remove_var("GITHUB_TOKEN");
        std::env::remove_var("GH_TOKEN");
    }

    #[test]
    fn test_resolve_token_from_github_token_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_token_env();
        std::env::set_var("GITHUB_TOKEN", "ghp_test123");
        let token = resolve_token().unwrap();
        assert_eq!(token, "ghp_test123");
        clear_token_env();
    }

    #[test]
    fn test_resolve_token_from_gh_token_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_token_env();
        std::env::set_var("GH_TOKEN", "ghp_fallback456");
        let token = resolve_token().unwrap();
        assert_eq!(token, "ghp_fallback456");
        clear_token_env();
    }

    #[test]
    fn test_resolve_token_prefers_github_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_token_env();
        std::env::set_var("GITHUB_TOKEN", "ghp_primary");
        std::env::set_var("GH_TOKEN", "ghp_secondary");
        let token = resolve_token().unwrap();
        assert_eq!(token, "ghp_primary");
        clear_token_env();
    }

    #[test]
    fn test_resolve_token_skips_empty() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_token_env();
        std::env::set_var("GITHUB_TOKEN", "");
        std::env::set_var("GH_TOKEN", "ghp_real");
        let token = resolve_token().unwrap();
        assert_eq!(token, "ghp_real");
        clear_token_env();
    }

    #[test]
    fn test_resolve_token_both_empty_falls_through() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_token_env();
        std::env::set_var("GITHUB_TOKEN", "");
        std::env::set_var("GH_TOKEN", "");
        // This will either succeed (gh CLI present) or fail (no gh CLI)
        // Either way, it should not panic
        let _result = resolve_token();
        clear_token_env();
    }

    #[test]
    fn test_auth_error_display() {
        let e = AuthError::NoToken;
        assert!(e.to_string().contains("no GitHub token"));

        let e = AuthError::GhCli("test error".into());
        assert!(e.to_string().contains("test error"));

        let e = AuthError::Timeout("timed out".into());
        assert!(e.to_string().contains("timed out"));
    }

    #[test]
    fn test_run_with_timeout_kills_hanging_child() {
        // Regression: resolve_token used to call .output() with no timeout.
        // A hanging gh would hang the entire MCP server. Verify the helper
        // kills children that exceed the deadline.
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let start = std::time::Instant::now();
        let result = run_with_timeout(cmd, Duration::from_millis(200));
        let elapsed = start.elapsed();

        assert!(matches!(result, Err(AuthError::Timeout(_))));
        // Should return well before sleep would have finished
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    }

    #[test]
    fn test_run_with_timeout_returns_stdout_on_success() {
        let mut cmd = Command::new("printf");
        cmd.arg("hello");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());

        let (success, stdout) = run_with_timeout(cmd, Duration::from_secs(5)).unwrap();
        assert!(success);
        assert_eq!(stdout, "hello");
    }

    #[test]
    fn test_run_with_timeout_reports_nonzero_exit() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 1"]);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let (success, _) = run_with_timeout(cmd, Duration::from_secs(5)).unwrap();
        assert!(!success);
    }
}
