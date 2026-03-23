/// URL-encode a string for use in GitHub API query parameters.
/// Encodes all characters except RFC 3986 unreserved characters: A-Z a-z 0-9 - _ . ~
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

/// Validate that a GitHub slug (owner, repo, branch) does not contain
/// path traversal or injection characters.
pub fn validate_slug(s: &str, name: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err(format!("invalid {name}: must not be empty"));
    }
    if s.contains("..") {
        return Err(format!("invalid {name}: contains path traversal"));
    }
    // owner and repo must not contain / (branch can for feature/foo patterns)
    if name == "owner" || name == "repo" {
        if s.contains('/') {
            return Err(format!("invalid {name}: contains '/'"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencode_spaces() {
        assert_eq!(urlencode("hello world"), "hello+world");
    }

    #[test]
    fn test_urlencode_special_chars() {
        assert_eq!(urlencode("a&b=c?d#e%f+g"), "a%26b%3Dc%3Fd%23e%25f%2Bg");
    }

    #[test]
    fn test_urlencode_passthrough() {
        assert_eq!(urlencode("simple"), "simple");
    }

    #[test]
    fn test_urlencode_non_ascii() {
        let encoded = urlencode("修复");
        assert!(!encoded.contains("修"));
        assert!(encoded.contains('%'));
    }

    #[test]
    fn test_urlencode_at_sign() {
        assert_eq!(urlencode("user@host"), "user%40host");
    }

    #[test]
    fn test_urlencode_brackets() {
        let encoded = urlencode("[test]");
        assert!(encoded.contains("%5B"));
        assert!(encoded.contains("%5D"));
    }

    #[test]
    fn test_urlencode_slash() {
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }

    #[test]
    fn test_urlencode_newline() {
        assert_eq!(urlencode("a\nb"), "a%0Ab");
    }

    #[test]
    fn test_validate_slug_ok() {
        assert!(validate_slug("octocat", "owner").is_ok());
        assert!(validate_slug("my-repo", "repo").is_ok());
        assert!(validate_slug("feature/foo", "branch").is_ok());
    }

    #[test]
    fn test_validate_slug_empty() {
        assert!(validate_slug("", "owner").is_err());
    }

    #[test]
    fn test_validate_slug_traversal() {
        assert!(validate_slug("../admin", "owner").is_err());
        assert!(validate_slug("foo/../bar", "repo").is_err());
    }

    #[test]
    fn test_validate_slug_slash_in_owner() {
        assert!(validate_slug("a/b", "owner").is_err());
    }

    #[test]
    fn test_validate_slug_slash_in_repo() {
        assert!(validate_slug("a/b", "repo").is_err());
    }

    #[test]
    fn test_validate_slug_slash_in_branch_ok() {
        assert!(validate_slug("feature/test", "branch").is_ok());
    }
}
