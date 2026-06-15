use std::io::{self, Seek};
use std::path::{Path, PathBuf};

/// 单次从日志文件尾部读取的最大字节数（1 MiB）。
const MAX_LOG_READ_BYTES: u64 = 1024 * 1024;

/// 在 logs 目录中找到最新的日志文件（按修改时间）
pub fn find_latest_log_file(logs_dir: &Path) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));
    entries.into_iter().next().map(|e| e.path())
}

/// 读取文件最后 n 行。
///
/// 实现从文件末尾向前 seek，最多只读 `MAX_LOG_READ_BYTES` 字节，避免大日志全量加载。
/// 若发生截断，返回结果的第一行固定为 `[...truncated]`。Windows CRLF 通过 `str::lines()`
/// 统一处理。
pub fn tail_lines(path: &Path, n: usize) -> anyhow::Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(MAX_LOG_READ_BYTES);

    file.seek(io::SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    io::Read::read_to_end(&mut file, &mut buf)?;

    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    let truncated = start > 0;

    // 若发生截断，先丢弃残缺的第一行；截断标记占据一行，因此实际日志取最后 n-1 行。
    let take = if truncated { n.saturating_sub(1) } else { n };
    let tail: Vec<String> = lines
        .into_iter()
        .skip(if truncated { 1 } else { 0 })
        .rev()
        .take(take)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    if truncated {
        let mut result = vec!["[...truncated]".to_string()];
        result.extend(tail);
        Ok(result)
    } else {
        Ok(tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tail_lines_basic_and_crlf() {
        let dir =
            std::env::temp_dir().join(format!("syncthing-test-tail-{}-basic", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");

        // 混合 LF 与 CRLF，验证 lines() 统一处理
        std::fs::write(&path, "line1\nline2\r\nline3\r\nline4\nline5").unwrap();
        let lines = tail_lines(&path, 3).unwrap();
        assert_eq!(lines, vec!["line3", "line4", "line5"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tail_lines_truncation() {
        let dir =
            std::env::temp_dir().join(format!("syncthing-test-tail-{}-trunc", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.log");

        // 构造一个远大于 1 MiB 的文件：每行约 100 字节，共 12000 行
        let line = "a".repeat(90);
        let mut content = String::with_capacity(12000 * 100);
        for i in 0..12000 {
            content.push_str(&format!("{} {}\n", i, line));
        }
        std::fs::write(&path, content).unwrap();

        let lines = tail_lines(&path, 50).unwrap();
        assert_eq!(lines.len(), 50);
        assert_eq!(lines[0], "[...truncated]");
        // 最后一行应是文件的最后一条完整日志
        assert!(lines.last().unwrap().starts_with("11999 "));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tail_lines_empty_file() {
        let dir =
            std::env::temp_dir().join(format!("syncthing-test-tail-{}-empty", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.log");
        std::fs::write(&path, "").unwrap();

        let lines = tail_lines(&path, 10).unwrap();
        assert!(lines.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
