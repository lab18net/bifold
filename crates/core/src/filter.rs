//! Quick-filter matching: plain text is a case-insensitive substring test,
//! `*` and `?` switch it to a glob.

pub fn matches(name: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return true;
    }
    let pat: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();

    if pat.iter().any(|c| *c == '*' || *c == '?') {
        glob(&pat, &name)
    } else {
        name.windows(pat.len()).any(|w| w == pat.as_slice())
    }
}

fn glob(pattern: &[char], name: &[char]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some('*') => glob(&pattern[1..], name) || (!name.is_empty() && glob(pattern, &name[1..])),
        Some('?') => !name.is_empty() && glob(&pattern[1..], &name[1..]),
        Some(c) => name.first() == Some(c) && glob(&pattern[1..], &name[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn empty_pattern_matches_everything() {
        assert!(matches("anything", ""));
        assert!(matches("anything", "   "));
    }

    #[test]
    fn substring_is_case_insensitive() {
        assert!(matches("Screenshot 2026-11-18.png", "screenshot"));
        assert!(matches("README.md", "EADM"));
        assert!(!matches("README.md", "readme.txt"));
    }

    #[test]
    fn globs() {
        assert!(matches("notes.txt", "*.txt"));
        assert!(matches("notes.txt", "n*"));
        assert!(matches("a.txt", "?.txt"));
        assert!(!matches("ab.txt", "?.txt"));
        assert!(matches("x", "*"));
        assert!(!matches("x", "?*?"));
    }
}
