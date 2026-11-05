//! `.stignore` parsing and matching.
//!
//! Implements Syncthing-style ignore rules:
//! - `*.ext` / `name` / `path/name` pattern matching
//! - `!pattern` negation (do not ignore)
//! - `/pattern` anchored to the folder root
//! - `pattern/` directories only
//! - `# comment` lines

use tracing::{debug, trace};

/// A single ignore rule.
#[derive(Debug, Clone)]
struct IgnoreRule {
    /// Match pattern with prefix/suffix modifiers stripped.
    pattern: String,
    /// Whether this is a negation rule (`!`).
    is_negation: bool,
    /// Whether the pattern is anchored to the folder root (`/`).
    anchored: bool,
    /// Whether the rule only matches directories (trailing `/`).
    directory_only: bool,
}

/// A set of `.stignore` rules.
#[derive(Debug, Clone, Default)]
pub struct IgnoreMatcher {
    rules: Vec<IgnoreRule>,
}

impl IgnoreMatcher {
    /// Creates an empty matcher.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Loads rules from a file path; a missing file yields an empty matcher.
    pub fn load(path: &std::path::Path) -> Self {
        let mut matcher = Self::new();
        match std::fs::read_to_string(path) {
            Ok(content) => {
                for line in content.lines() {
                    matcher.add_line(line);
                }
                debug!(path = %path.display(), rules = matcher.rules.len(), "Loaded .stignore");
            }
            Err(e) => {
                trace!(path = %path.display(), error = %e, "No .stignore found");
            }
        }
        matcher
    }

    /// Parses and adds a single rule line.
    pub fn add_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return;
        }

        let mut pattern = line;
        let is_negation = pattern.starts_with('!');
        if is_negation {
            pattern = &pattern[1..];
        }

        let directory_only = pattern.ends_with('/');
        if directory_only {
            pattern = &pattern[..pattern.len() - 1];
        }

        let anchored = pattern.starts_with('/');
        if anchored {
            pattern = &pattern[1..];
        }

        self.rules.push(IgnoreRule {
            pattern: pattern.to_string(),
            is_negation,
            anchored,
            directory_only,
        });
    }

    /// Returns `true` when the given relative path should be ignored.
    ///
    /// `relative_path` uses `/` separators (e.g. `foo/bar.txt`); `is_dir`
    /// indicates whether the path is a directory. Later rules override
    /// earlier ones for the same path.
    pub fn matches(&self, relative_path: &str, is_dir: bool) -> bool {
        let mut ignored = false;

        for rule in &self.rules {
            if rule.directory_only && !is_dir {
                continue;
            }

            let matched = if rule.anchored {
                anchored_match(&rule.pattern, relative_path)
            } else {
                unanchored_match(&rule.pattern, relative_path)
            };

            if matched {
                ignored = !rule.is_negation;
            }
        }

        ignored
    }

    /// Returns `true` when no rules have been loaded.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Returns the number of loaded rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }
}

/// Simple glob matching: `*` matches any sequence of characters (excluding
/// `/`), `?` matches a single character.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat = pattern.as_bytes();
    let txt = text.as_bytes();
    let mut p = 0usize;
    let mut t = 0usize;
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0usize;

    while t < txt.len() {
        if p < pat.len() && (pat[p] == txt[t] || pat[p] == b'?') {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star_idx = Some(p);
            match_idx = t;
            p += 1;
        } else if let Some(star) = star_idx {
            p = star + 1;
            match_idx += 1;
            t = match_idx;
        } else {
            return false;
        }
    }

    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }

    p == pat.len()
}

/// Unanchored matching: the pattern may match at any level of the path.
fn unanchored_match(pattern: &str, path: &str) -> bool {
    let path_components: Vec<&str> = path.split('/').collect();

    if pattern.contains('/') {
        let pat_components: Vec<&str> = pattern.split('/').collect();
        if pat_components.len() > path_components.len() {
            return false;
        }
        for start in 0..=path_components.len() - pat_components.len() {
            if pat_components
                .iter()
                .zip(&path_components[start..])
                .all(|(p, c)| glob_match(p, c))
            {
                return true;
            }
        }
        false
    } else {
        path_components.iter().any(|c| glob_match(pattern, c))
    }
}

/// Anchored matching: the pattern must match starting at the path root.
fn anchored_match(pattern: &str, path: &str) -> bool {
    let pat_components: Vec<&str> = pattern.split('/').collect();
    let path_components: Vec<&str> = path.split('/').collect();

    if pat_components.len() > path_components.len() {
        return false;
    }

    pat_components
        .iter()
        .zip(path_components.iter())
        .all(|(p, c)| glob_match(p, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_ignore() {
        let mut m = IgnoreMatcher::new();
        m.add_line("*.tmp");
        assert!(m.matches("foo.tmp", false));
        assert!(!m.matches("foo.txt", false));
    }

    #[test]
    fn test_negation() {
        let mut m = IgnoreMatcher::new();
        m.add_line("*.txt");
        m.add_line("!important.txt");
        assert!(m.matches("foo.txt", false));
        assert!(!m.matches("important.txt", false));
    }

    #[test]
    fn test_anchored() {
        let mut m = IgnoreMatcher::new();
        m.add_line("/build");
        assert!(m.matches("build", true));
        assert!(!m.matches("src/build", true));
    }

    #[test]
    fn test_directory_only() {
        let mut m = IgnoreMatcher::new();
        m.add_line("target/");
        assert!(m.matches("target", true));
        assert!(!m.matches("target", false));
    }

    #[test]
    fn test_path_pattern() {
        let mut m = IgnoreMatcher::new();
        m.add_line("node_modules");
        assert!(m.matches("node_modules", true));
        assert!(m.matches("src/node_modules", true));
    }

    #[test]
    fn test_comments_and_empty() {
        let mut m = IgnoreMatcher::new();
        m.add_line("# this is a comment");
        m.add_line("");
        m.add_line("*.log");
        assert!(m.matches("debug.log", false));
        assert_eq!(m.rules.len(), 1);
    }

    #[test]
    fn test_hash_comment() {
        let mut m = IgnoreMatcher::new();
        m.add_line("# this is a standard syncthing comment");
        m.add_line("*.log");
        m.add_line("# another comment");
        assert!(m.matches("debug.log", false));
        assert!(!m.matches("debug.txt", false));
        assert_eq!(m.rules.len(), 1);
    }

    #[test]
    fn test_skills_directory_exclusion() {
        let mut m = IgnoreMatcher::new();
        m.add_line("skills/");
        m.add_line("tools/");
        m.add_line("vault/");

        // 目录应被排除
        assert!(
            m.matches("skills", true),
            "skills/ should match skills directory"
        );
        assert!(
            m.matches("tools", true),
            "tools/ should match tools directory"
        );
        assert!(
            m.matches("vault", true),
            "vault/ should match vault directory"
        );

        // 同名文件不应被排除
        assert!(
            !m.matches("skills", false),
            "skills/ should NOT match skills file"
        );
        assert!(
            !m.matches("tools", false),
            "tools/ should NOT match tools file"
        );

        // 子目录中的同名目录也应被排除
        assert!(
            m.matches("workspace/skills", true),
            "skills/ should match workspace/skills directory"
        );
        assert!(
            m.matches("deep/nested/vault", true),
            "vault/ should match deep/nested/vault directory"
        );
    }

    #[test]
    fn test_load_from_lines_with_hash_comments() {
        let content = r#"# Syncthing ignore patterns
*.tmp
# Build directories
target/

# IDE files
.idea/
"#;

        let mut m = IgnoreMatcher::new();
        for line in content.lines() {
            m.add_line(line);
        }

        assert_eq!(m.len(), 3);
        assert!(m.matches("foo.tmp", false));
        assert!(m.matches("target", true));
        assert!(m.matches(".idea", true));
        assert!(!m.matches("foo.txt", false));
    }
}
