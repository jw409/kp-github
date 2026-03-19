use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("no GitHub token found: set GITHUB_TOKEN, GH_TOKEN, or install gh CLI")]
    NoToken,
    #[error("gh CLI auth failed: {0}")]
    GhCli(String),
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

    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| AuthError::GhCli(e.to_string()))?;

    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    Err(AuthError::NoToken)
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
    }
}
