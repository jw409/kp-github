use serde::Deserialize;

#[allow(dead_code)] // Used in tests; kept for future Phase 4 query planner
#[derive(Debug, Clone, Deserialize)]
pub struct RepoRef {
    pub owner: String,
    pub repo: String,
}

#[allow(dead_code)]
impl RepoRef {
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    /// Parse "owner/repo" format
    pub fn parse(s: &str) -> Option<Self> {
        let (owner, repo) = s.split_once('/')?;
        Some(Self::new(owner, repo))
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Open,
    Closed,
    All,
}

#[allow(dead_code)]
impl State {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- RepoRef ---

    #[test]
    fn test_repo_ref_new() {
        let r = RepoRef::new("owner", "repo");
        assert_eq!(r.owner, "owner");
        assert_eq!(r.repo, "repo");
    }

    #[test]
    fn test_repo_ref_new_from_string() {
        let r = RepoRef::new(String::from("alice"), String::from("project"));
        assert_eq!(r.owner, "alice");
        assert_eq!(r.repo, "project");
    }

    #[test]
    fn test_repo_ref_parse_valid() {
        let r = RepoRef::parse("owner/repo").unwrap();
        assert_eq!(r.owner, "owner");
        assert_eq!(r.repo, "repo");
    }

    #[test]
    fn test_repo_ref_parse_with_dots_and_dashes() {
        let r = RepoRef::parse("my-org/my-repo.rs").unwrap();
        assert_eq!(r.owner, "my-org");
        assert_eq!(r.repo, "my-repo.rs");
    }

    #[test]
    fn test_repo_ref_parse_no_slash() {
        assert!(RepoRef::parse("noslash").is_none());
    }

    #[test]
    fn test_repo_ref_parse_empty() {
        assert!(RepoRef::parse("").is_none());
    }

    #[test]
    fn test_repo_ref_parse_multiple_slashes() {
        // split_once takes first slash only
        let r = RepoRef::parse("a/b/c").unwrap();
        assert_eq!(r.owner, "a");
        assert_eq!(r.repo, "b/c");
    }

    #[test]
    fn test_repo_ref_deserialize() {
        let json = r#"{"owner": "jw409", "repo": "whiteout"}"#;
        let r: RepoRef = serde_json::from_str(json).unwrap();
        assert_eq!(r.owner, "jw409");
        assert_eq!(r.repo, "whiteout");
    }

    #[test]
    fn test_repo_ref_clone() {
        let r1 = RepoRef::new("a", "b");
        let r2 = r1.clone();
        assert_eq!(r2.owner, "a");
        assert_eq!(r2.repo, "b");
    }

    // --- State ---

    #[test]
    fn test_state_as_str_open() {
        assert_eq!(State::Open.as_str(), "open");
    }

    #[test]
    fn test_state_as_str_closed() {
        assert_eq!(State::Closed.as_str(), "closed");
    }

    #[test]
    fn test_state_as_str_all() {
        assert_eq!(State::All.as_str(), "all");
    }

    #[test]
    fn test_state_deserialize_open() {
        let s: State = serde_json::from_str(r#""open""#).unwrap();
        assert_eq!(s.as_str(), "open");
    }

    #[test]
    fn test_state_deserialize_closed() {
        let s: State = serde_json::from_str(r#""closed""#).unwrap();
        assert_eq!(s.as_str(), "closed");
    }

    #[test]
    fn test_state_deserialize_all() {
        let s: State = serde_json::from_str(r#""all""#).unwrap();
        assert_eq!(s.as_str(), "all");
    }

    #[test]
    fn test_state_deserialize_invalid() {
        let result: Result<State, _> = serde_json::from_str(r#""invalid""#);
        assert!(result.is_err());
    }

    #[test]
    fn test_state_copy() {
        let s = State::Open;
        let s2 = s; // Copy
        assert_eq!(s.as_str(), s2.as_str());
    }

    // --- SortDirection ---

    #[test]
    fn test_sort_direction_deserialize_asc() {
        let d: SortDirection = serde_json::from_str(r#""asc""#).unwrap();
        matches!(d, SortDirection::Asc);
    }

    #[test]
    fn test_sort_direction_deserialize_desc() {
        let d: SortDirection = serde_json::from_str(r#""desc""#).unwrap();
        matches!(d, SortDirection::Desc);
    }

    #[test]
    fn test_sort_direction_deserialize_invalid() {
        let result: Result<SortDirection, _> = serde_json::from_str(r#""sideways""#);
        assert!(result.is_err());
    }
}
