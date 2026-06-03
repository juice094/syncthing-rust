use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_empty_patterns() {
    let patterns = IgnorePatterns::new();
    assert!(!patterns.is_ignored(Path::new("anything.txt")));
}

#[test]
fn test_simple_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();

    assert!(patterns.is_ignored(Path::new("file.txt")));
    assert!(patterns.is_ignored(Path::new("dir/file.txt")));
    assert!(!patterns.is_ignored(Path::new("file.doc")));
}

#[test]
fn test_directory_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("build/").unwrap();

    assert!(patterns.is_ignored(Path::new("build")));
    assert!(patterns.is_ignored(Path::new("build/output")));
    assert!(!patterns.is_ignored(Path::new("build.txt")));
}

#[test]
fn test_include_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();
    patterns.add_pattern("!important.txt").unwrap();

    assert!(patterns.is_ignored(Path::new("file.txt")));
    assert!(!patterns.is_ignored(Path::new("important.txt")));
}

#[test]
fn test_root_only_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("/root.txt").unwrap();

    assert!(patterns.is_ignored(Path::new("root.txt")));
    assert!(!patterns.is_ignored(Path::new("subdir/root.txt")));
}

#[test]
fn test_double_star_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("**/temp").unwrap();

    assert!(patterns.is_ignored(Path::new("temp")));
    assert!(patterns.is_ignored(Path::new("a/temp")));
    assert!(patterns.is_ignored(Path::new("a/b/temp")));
}

#[test]
fn test_question_mark_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("file?.txt").unwrap();

    assert!(patterns.is_ignored(Path::new("file1.txt")));
    assert!(patterns.is_ignored(Path::new("fileA.txt")));
    assert!(!patterns.is_ignored(Path::new("file12.txt")));
}

#[test]
fn test_from_string() {
    let content = r#"
# This is a comment
*.log
build/
!important.log
"#;

    let patterns = IgnorePatterns::parse(content);

    assert!(patterns.is_ignored(Path::new("debug.log")));
    assert!(patterns.is_ignored(Path::new("build")));
    assert!(!patterns.is_ignored(Path::new("important.log")));
}

#[tokio::test]
async fn test_from_file() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "*.tmp").unwrap();
    writeln!(temp_file, "temp/").unwrap();

    let patterns = IgnorePatterns::from_file(temp_file.path()).await.unwrap();

    assert!(patterns.is_ignored(Path::new("file.tmp")));
    assert!(patterns.is_ignored(Path::new("temp")));
}

#[test]
fn test_default_patterns() {
    let patterns = default_ignore_patterns();

    assert!(patterns.is_ignored(Path::new(".stfolder")));
    assert!(patterns.is_ignored(Path::new(".stignore")));
    assert!(patterns.is_ignored(Path::new(".DS_Store")));
    assert!(patterns.is_ignored(Path::new("Thumbs.db")));
}

#[test]
fn test_windows_paths() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();

    // Windows path separators should be normalized
    assert!(patterns.is_ignored(Path::new("dir\\file.txt")));
}

#[test]
fn test_last_match_wins() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();
    patterns.add_pattern("!keep.txt").unwrap();
    patterns.add_pattern("keep.txt").unwrap(); // This re-ignores it

    assert!(patterns.is_ignored(Path::new("keep.txt")));
}

#[test]
fn test_escaped_characters() {
    let mut patterns = IgnorePatterns::new();
    // Note: backslash escaping is handled in pattern parsing
    patterns.add_pattern("file\\.txt").unwrap();

    assert!(patterns.is_ignored(Path::new("file.txt")));
}

#[test]
fn test_is_included() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();
    patterns.add_pattern("!special.txt").unwrap();

    assert!(patterns.is_included(Path::new("special.txt")));
    assert!(!patterns.is_included(Path::new("other.txt")));
}

#[test]
fn test_pattern_count() {
    let mut patterns = IgnorePatterns::new();
    assert_eq!(patterns.len(), 0);
    assert!(patterns.is_empty());

    patterns.add_pattern("*.txt").unwrap();
    assert_eq!(patterns.len(), 1);
    assert!(!patterns.is_empty());
}

#[test]
fn test_case_insensitive_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("(?i)*.TXT").unwrap();

    assert!(patterns.is_ignored(Path::new("file.txt")));
    assert!(patterns.is_ignored(Path::new("file.TXT")));
    assert!(patterns.is_ignored(Path::new("file.Txt")));
    assert!(!patterns.is_ignored(Path::new("file.doc")));
}

#[test]
fn test_case_insensitive_prefix_order() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();
    patterns.add_pattern("!(?i)KEEP.txt").unwrap();

    assert!(patterns.is_ignored(Path::new("other.txt")));
    assert!(!patterns.is_ignored(Path::new("keep.txt")));
    assert!(!patterns.is_ignored(Path::new("KEEP.TXT")));
}

#[test]
fn test_allow_delete_pattern() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("(?d)temp/").unwrap();

    assert!(patterns.is_ignored(Path::new("temp")));
    assert!(!patterns.allows_skipping_ignored_dirs());
}

#[test]
fn test_combined_prefixes() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("(?i)(?d)secret").unwrap();

    assert!(patterns.is_ignored(Path::new("SECRET")));
    assert!(patterns.is_ignored(Path::new("secret")));
    assert!(!patterns.allows_skipping_ignored_dirs());
}

#[tokio::test]
async fn test_include_recursive() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let root = temp_dir.path();

    let main_path = root.join(".stignore");
    tokio::fs::write(&main_path, "*.log\n#include sub.ignore\n")
        .await
        .unwrap();

    let sub_path = root.join("sub.ignore");
    tokio::fs::write(&sub_path, "*.tmp\n").await.unwrap();

    let patterns = IgnorePatterns::from_file(&main_path).await.unwrap();

    assert!(patterns.is_ignored(Path::new("debug.log")));
    assert!(patterns.is_ignored(Path::new("file.tmp")));
    assert!(!patterns.is_ignored(Path::new("file.txt")));
}

#[tokio::test]
async fn test_at_include() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let root = temp_dir.path();

    let main_path = root.join(".stignore");
    tokio::fs::write(&main_path, "*.log\n@include other.ignore\n")
        .await
        .unwrap();

    let other_path = root.join("other.ignore");
    tokio::fs::write(&other_path, "*.bak\n").await.unwrap();

    let patterns = IgnorePatterns::from_file(&main_path).await.unwrap();

    assert!(patterns.is_ignored(Path::new("debug.log")));
    assert!(patterns.is_ignored(Path::new("file.bak")));
}

#[tokio::test]
async fn test_include_cycle_detection() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let root = temp_dir.path();

    let main_path = root.join(".stignore");
    tokio::fs::write(&main_path, "*.log\n#include a.ignore\n")
        .await
        .unwrap();

    let a_path = root.join("a.ignore");
    tokio::fs::write(&a_path, "*.tmp\n#include b.ignore\n")
        .await
        .unwrap();

    let b_path = root.join("b.ignore");
    tokio::fs::write(&b_path, "*.bak\n#include a.ignore\n")
        .await
        .unwrap();

    let patterns = IgnorePatterns::from_file(&main_path).await.unwrap();

    // Should not infinite loop; patterns from a and b should be loaded once
    assert!(patterns.is_ignored(Path::new("debug.log")));
    assert!(patterns.is_ignored(Path::new("file.tmp")));
    assert!(patterns.is_ignored(Path::new("file.bak")));
}

#[test]
fn test_cache_hit_and_invalidation() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();

    // First calls populate cache
    assert!(patterns.is_ignored_cached(Path::new("file.txt")));
    assert!(!patterns.is_ignored_cached(Path::new("file.doc")));

    // Clear cache
    patterns.clear_cache();

    // Should still work after clearing
    assert!(patterns.is_ignored_cached(Path::new("file.txt")));
    assert!(!patterns.is_ignored_cached(Path::new("file.doc")));
}

#[test]
fn test_allows_skipping_ignored_dirs() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("*.txt").unwrap();
    assert!(patterns.allows_skipping_ignored_dirs());

    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("!important.txt").unwrap();
    assert!(!patterns.allows_skipping_ignored_dirs());

    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("(?d)temp/").unwrap();
    assert!(!patterns.allows_skipping_ignored_dirs());
}

/// Audit: verify common real-world scenarios
#[test]
fn test_real_world_scenarios() {
    let content = r#"
# Node modules
node_modules/

# Build outputs
dist/
build/
target/

# IDE
.idea/
.vscode/

# VCS
.git/

# Logs
*.log

# Temp
*.tmp
*.temp

# Keep specific files
!important.log
"#;
    let patterns = IgnorePatterns::parse(content);

    assert!(patterns.is_ignored(Path::new("node_modules")));
    assert!(patterns.is_ignored(Path::new("node_modules/lodash/index.js")));
    assert!(patterns.is_ignored(Path::new("dist/bundle.js")));
    assert!(patterns.is_ignored(Path::new("target/debug/main.exe")));
    assert!(patterns.is_ignored(Path::new(".idea/workspace.xml")));
    assert!(patterns.is_ignored(Path::new(".git/config")));
    assert!(patterns.is_ignored(Path::new("debug.log")));
    assert!(!patterns.is_ignored(Path::new("important.log")));
    assert!(!patterns.is_ignored(Path::new("src/main.rs")));
}

/// Audit: a/**/b pattern (intermediate double-star)
/// Current implementation: **/ consumes at most one directory level.
/// This is a known gap vs Go Syncthing / gitignore semantics.
#[test]
fn test_intermediate_double_star_gap() {
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("src/**/test.rs").unwrap();

    // These should match (and do)
    assert!(patterns.is_ignored(Path::new("src/test.rs")));
    assert!(patterns.is_ignored(Path::new("src/a/test.rs")));

    // Go Syncthing matches arbitrary depth — now fixed
    assert!(patterns.is_ignored(Path::new("src/a/b/test.rs"))); // was FIXME: now works
}

/// Go Syncthing only supports # comments, not //.
/// `//build` is NOT a comment — it's parsed as a root-only pattern `/build`.
/// The first / is is_root_only flag, and build is the pattern.
#[test]
fn test_double_slash_not_a_comment() {
    // Parse: //build → is_root_only + pattern "build" (second / consumed, leaving build)
    let mut patterns = IgnorePatterns::new();
    patterns.add_pattern("//build").unwrap();
    assert!(patterns.is_ignored(Path::new("build")));
    assert!(!patterns.is_ignored(Path::new("src/build")));

    // Also verify that // is not treated as comment in Parse
    let content = "//build\n*.tmp";
    let parsed = IgnorePatterns::parse(content);
    assert!(parsed.is_ignored(Path::new("build")));
    assert!(parsed.is_ignored(Path::new("file.tmp")));
    assert_eq!(parsed.len(), 2, "expected 2 patterns (not a comment)");
}
