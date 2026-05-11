#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_native_filesystem_new() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());
        assert_eq!(fs.root, temp_dir.path());
    }

    #[tokio::test]
    async fn test_create_dir_and_exists() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_dir = Path::new("test_subdir");
        assert!(!fs.exists(test_dir).await.unwrap());

        fs.create_dir(test_dir).await.unwrap();
        assert!(fs.exists(test_dir).await.unwrap());
    }

    #[tokio::test]
    async fn test_write_and_read_block() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_file = Path::new("test.txt");
        let data = b"Hello, World!";

        fs.write_block(test_file, 0, data).await.unwrap();

        let read_data = fs.read_block(test_file, 0, 100).await.unwrap();
        assert_eq!(read_data, data);
    }

    #[tokio::test]
    async fn test_write_block_at_offset() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_file = Path::new("test.txt");

        // Write first part
        fs.write_block(test_file, 0, b"Hello").await.unwrap();
        // Write second part at offset
        fs.write_block(test_file, 5, b" World").await.unwrap();

        let read_data = fs.read_block(test_file, 0, 100).await.unwrap();
        assert_eq!(read_data, b"Hello World");
    }

    #[tokio::test]
    async fn test_read_block_partial() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_file = Path::new("test.txt");
        fs.write_block(test_file, 0, b"Hello, World!")
            .await
            .unwrap();

        let read_data = fs.read_block(test_file, 7, 5).await.unwrap();
        assert_eq!(read_data, b"World");
    }

    #[tokio::test]
    async fn test_file_info() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_file = Path::new("test.txt");
        let data = b"Test content for hashing";
        fs.write_block(test_file, 0, data).await.unwrap();

        let info = fs.file_info(test_file).await.unwrap();
        assert_eq!(info.name, "test.txt");
        assert_eq!(info.size, data.len() as i64);
        assert_eq!(info.file_type, FileType::File);
    }

    #[tokio::test]
    async fn test_remove() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_file = Path::new("to_delete.txt");
        fs.write_block(test_file, 0, b"delete me").await.unwrap();
        assert!(fs.exists(test_file).await.unwrap());

        fs.remove(test_file).await.unwrap();
        assert!(!fs.exists(test_file).await.unwrap());
    }

    #[tokio::test]
    async fn test_rename() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let from = Path::new("old_name.txt");
        let to = Path::new("new_name.txt");

        fs.write_block(from, 0, b"content").await.unwrap();
        fs.rename(from, to).await.unwrap();

        assert!(!fs.exists(from).await.unwrap());
        assert!(fs.exists(to).await.unwrap());

        let content = fs.read_block(to, 0, 100).await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_scan_directory() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        // Create some files and directories
        fs.create_dir(Path::new("subdir")).await.unwrap();
        fs.write_block(Path::new("file1.txt"), 0, b"content1")
            .await
            .unwrap();
        fs.write_block(Path::new("subdir/file2.txt"), 0, b"content2")
            .await
            .unwrap();

        let entries = fs.scan_directory(Path::new(".")).await.unwrap();
        assert_eq!(entries.len(), 3); // subdir, file1.txt, subdir/file2.txt

        let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"subdir".to_string()));
        assert!(names.contains(&"file1.txt".to_string()));
        assert!(names.contains(&"subdir/file2.txt".to_string()));
    }

    #[tokio::test]
    async fn test_scan_directory_with_ignore() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        // Create some files and directories
        fs.create_dir(Path::new("ignored_dir")).await.unwrap();
        fs.create_dir(Path::new("kept_dir")).await.unwrap();
        fs.write_block(Path::new("ignored.txt"), 0, b"ignored")
            .await
            .unwrap();
        fs.write_block(Path::new("kept.txt"), 0, b"kept")
            .await
            .unwrap();
        fs.write_block(Path::new("ignored_dir/nested.txt"), 0, b"nested")
            .await
            .unwrap();
        fs.write_block(Path::new("kept_dir/nested.txt"), 0, b"nested")
            .await
            .unwrap();

        let mut patterns = IgnorePatterns::new();
        patterns.add_pattern("ignored_dir/").unwrap();
        patterns.add_pattern("ignored.txt").unwrap();

        let entries = fs
            .scan_directory_with_ignore(Path::new("."), &patterns)
            .await
            .unwrap();

        let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        assert!(!names.contains(&"ignored_dir".to_string()));
        assert!(!names.contains(&"ignored_dir/nested.txt".to_string()));
        assert!(!names.contains(&"ignored.txt".to_string()));
        assert!(names.contains(&"kept_dir".to_string()));
        assert!(names.contains(&"kept_dir/nested.txt".to_string()));
        assert!(names.contains(&"kept.txt".to_string()));
    }

    #[tokio::test]
    async fn test_hash_file() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let test_file = Path::new("hash_me.txt");
        let data = b"data to hash";
        fs.write_block(test_file, 0, data).await.unwrap();

        let hashes = fs.hash_file(test_file).await.unwrap();
        assert_eq!(hashes.len(), 1);

        // Verify hash is correct
        let expected_hash = BlockHash::from_data(data);
        assert_eq!(hashes[0], expected_hash);
    }

    #[tokio::test]
    async fn test_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("atomic.txt");

        atomic_write(&path, b"atomic content").await.unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "atomic content");
    }

    #[tokio::test]
    async fn test_nested_directory_creation() {
        let temp_dir = TempDir::new().unwrap();
        let fs = NativeFileSystem::new(temp_dir.path());

        let nested_file = Path::new("a/b/c/nested.txt");
        fs.write_block(nested_file, 0, b"nested content")
            .await
            .unwrap();

        let content = fs.read_block(nested_file, 0, 100).await.unwrap();
        assert_eq!(content, b"nested content");
    }
}
