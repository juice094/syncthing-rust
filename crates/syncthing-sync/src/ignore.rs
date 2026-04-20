//! `.stignore` 解析与匹配
//!
//! 实现 Syncthing 风格的 ignore 规则，支持：
//! - `*.ext` / `name` / `path/name` 模式匹配
//! - `!pattern` 否定规则（不忽略）
//! - `/pattern` 锚定到根目录
//! - `pattern/` 仅匹配目录
//! - `// comment` 注释

use tracing::{debug, trace};

/// 单条 ignore 规则
#[derive(Debug, Clone)]
struct IgnoreRule {
    /// 匹配模式（已去除前缀/后缀修饰符）
    pattern: String,
    /// 是否为否定规则（!）
    is_negation: bool,
    /// 是否锚定到根目录（/）
    anchored: bool,
    /// 是否仅匹配目录（以 / 结尾）
    directory_only: bool,
}

/// `.stignore` 规则集
#[derive(Debug, Clone, Default)]
pub struct IgnoreMatcher {
    rules: Vec<IgnoreRule>,
}

impl IgnoreMatcher {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// 从文件路径加载规则
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

    /// 解析单行规则
    pub fn add_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
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

    /// 判断给定相对路径是否应被忽略
    ///
    /// `relative_path` 使用 `/` 分隔（如 `foo/bar.txt`）
    /// `is_dir` 表示该路径是否为目录
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

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// 简单 glob 匹配：* 匹配任意字符序列（不含 /），? 匹配单个字符
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

/// 非锚定匹配：模式可以匹配路径的任何层级
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

/// 锚定匹配：模式必须从路径的根开始匹配
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
        m.add_line("// this is a comment");
        m.add_line("");
        m.add_line("*.log");
        assert!(m.matches("debug.log", false));
        assert_eq!(m.rules.len(), 1);
    }
}
