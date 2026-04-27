/// URL-encode a string for use in GitHub API query parameters
/// (`application/x-www-form-urlencoded`). Spaces become `+`.
///
/// **Do not use for URL path segments** — `+` is a literal `+` in a path,
/// not a space, so a query encoder applied to a path component produces
/// 404s on GitHub (e.g. label `priority: P0`). Use [`urlencode_path`] or
/// [`urlencode_path_multi`] for path components.
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

/// Percent-encode a single URL path segment per RFC 3986.
///
/// Encodes everything except unreserved characters (`A-Z a-z 0-9 - _ . ~`).
/// Spaces become `%20`. Slashes are encoded as `%2F` — use this when the
/// value is one path segment and a literal `/` would split the segment
/// (e.g. a label name, team slug).
pub fn urlencode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

/// Percent-encode a multi-segment URL path component per RFC 3986, preserving `/`.
///
/// Same as [`urlencode_path`] but `/` passes through unencoded so callers
/// can pass values that legitimately span path segments — file paths
/// (`src/foo bar.rs`), branch refs (`feature/foo`), git refs in
/// `compare/{base}...{head}`. Spaces become `%20`.
pub fn urlencode_path_multi(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char);
            }
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

    // ---- urlencode_path: single path segment, RFC 3986 ----

    #[test]
    fn test_urlencode_path_space_is_percent_20() {
        // The bug from issue #19: space must be %20 in a path, not +
        assert_eq!(urlencode_path("hello world"), "hello%20world");
    }

    #[test]
    fn test_urlencode_path_label_with_colon_and_space() {
        // Exact repro from issue #19: GitHub label "priority: P0"
        assert_eq!(urlencode_path("priority: P0"), "priority%3A%20P0");
    }

    #[test]
    fn test_urlencode_path_passthrough_unreserved() {
        assert_eq!(urlencode_path("v1.0.0-alpha_beta~final"), "v1.0.0-alpha_beta~final");
    }

    #[test]
    fn test_urlencode_path_special_chars() {
        assert_eq!(urlencode_path("a&b=c?d#e%f+g"), "a%26b%3Dc%3Fd%23e%25f%2Bg");
    }

    #[test]
    fn test_urlencode_path_encodes_slash() {
        // A single segment must not let / escape — prevents path injection.
        assert_eq!(urlencode_path("a/b"), "a%2Fb");
    }

    #[test]
    fn test_urlencode_path_non_ascii() {
        // UTF-8 multi-byte characters get percent-encoded byte-by-byte.
        let encoded = urlencode_path("修复");
        assert!(!encoded.contains('修'));
        assert!(encoded.starts_with('%'));
        // Each Chinese char is 3 UTF-8 bytes → 9 chars of %XX each → 18 chars total
        assert_eq!(encoded.len(), 18);
    }

    #[test]
    fn test_urlencode_path_brackets_and_parens() {
        assert_eq!(urlencode_path("[test]"), "%5Btest%5D");
        assert_eq!(urlencode_path("(x)"), "%28x%29");
    }

    #[test]
    fn test_urlencode_path_no_literal_space_or_plus_for_space() {
        // Regression guard: never emit `+` or a literal space for space input.
        for input in ["a b", " ", "  ", "x y z"] {
            let out = urlencode_path(input);
            assert!(!out.contains(' '), "literal space in {out:?}");
            assert!(!out.contains('+'), "form-style + for space in {out:?}");
            assert!(out.contains("%20"), "expected %20 in {out:?}");
        }
    }

    // ---- urlencode_path_multi: multi-segment path, preserves / ----

    #[test]
    fn test_urlencode_path_multi_preserves_slash() {
        assert_eq!(urlencode_path_multi("feature/foo"), "feature/foo");
    }

    #[test]
    fn test_urlencode_path_multi_space_is_percent_20() {
        assert_eq!(urlencode_path_multi("src/foo bar.rs"), "src/foo%20bar.rs");
    }

    #[test]
    fn test_urlencode_path_multi_branch_with_space() {
        // A branch name like `feature/my fix` is rare but legal.
        assert_eq!(urlencode_path_multi("feature/my fix"), "feature/my%20fix");
    }

    #[test]
    fn test_urlencode_path_multi_special_chars() {
        // ? and # would otherwise terminate the path; must be encoded.
        assert_eq!(urlencode_path_multi("a/b?c#d"), "a/b%3Fc%23d");
    }

    #[test]
    fn test_urlencode_path_multi_does_not_double_encode_slash() {
        // Trailing/leading slashes should pass through.
        assert_eq!(urlencode_path_multi("/a/b/"), "/a/b/");
    }

    // ---- Cross-encoder invariant ----

    #[test]
    fn test_path_encoders_never_produce_form_plus_for_space() {
        // The whole class of bug from issue #19: form-style + for space
        // is invalid in a URL path. Both path encoders must avoid it.
        let inputs = [
            "priority: P0",
            "help wanted",
            "good first issue",
            "src/with space/file.rs",
        ];
        for input in inputs {
            let single = urlencode_path(input);
            let multi = urlencode_path_multi(input);
            assert!(!single.contains('+'), "urlencode_path({input:?}) → {single}");
            assert!(!multi.contains('+'), "urlencode_path_multi({input:?}) → {multi}");
        }
    }
}
