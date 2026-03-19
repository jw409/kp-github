/// URL-encode a string for use in GitHub API query parameters.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push('+'),
            '&' => out.push_str("%26"),
            '#' => out.push_str("%23"),
            '=' => out.push_str("%3D"),
            '?' => out.push_str("%3F"),
            '%' => out.push_str("%25"),
            '+' => out.push_str("%2B"),
            _ => out.push(c),
        }
    }
    out
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
}
