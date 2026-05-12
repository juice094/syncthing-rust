use super::*;

#[test]
fn test_vector_compare_equal() {
    let a = Vector::new().with_counter(1, 5);
    let b = Vector::new().with_counter(1, 5);
    assert_eq!(a.compare(&b), VersionComparison::Equal);
}

#[test]
fn test_vector_compare_greater() {
    let a = Vector::new().with_counter(1, 5);
    let b = Vector::new().with_counter(1, 3);
    assert_eq!(a.compare(&b), VersionComparison::Greater);
}

#[test]
fn test_vector_compare_less() {
    let a = Vector::new().with_counter(1, 3);
    let b = Vector::new().with_counter(1, 5);
    assert_eq!(a.compare(&b), VersionComparison::Less);
}

#[test]
fn test_vector_compare_conflict() {
    let a = Vector::new().with_counter(1, 5).with_counter(2, 3);
    let b = Vector::new().with_counter(1, 3).with_counter(2, 7);
    assert_eq!(a.compare(&b), VersionComparison::Conflict);
}

#[test]
fn test_vector_dominates() {
    let a = Vector::new().with_counter(1, 5).with_counter(2, 3);
    let b = Vector::new().with_counter(1, 3).with_counter(2, 3);
    assert!(a.dominates(&b));
    assert!(!b.dominates(&a));
}

#[test]
fn test_vector_increment() {
    let mut v = Vector::new();
    v.increment(42);
    assert_eq!(v.get(42), 1);
    v.increment(42);
    assert_eq!(v.get(42), 2);
}

#[test]
fn test_index_id_roundtrip() {
    let original = IndexID::from_u64(0x123456789ABCDEF0);
    assert_eq!(original.as_u64(), 0x123456789ABCDEF0);
}

#[test]
fn test_folder_summary_synced() {
    let mut summary = FolderSummary::default();
    assert!(summary.is_synced());
    summary.need_files = 1;
    assert!(!summary.is_synced());
}

#[test]
fn test_folder_summary_sync_percent() {
    let summary = FolderSummary {
        bytes: 1000,
        need_bytes: 250,
        ..Default::default()
    };
    assert_eq!(summary.sync_percent(), 75.0);
}

#[test]
fn test_folder_summary_sync_percent_zero_bytes() {
    let summary = FolderSummary::default();
    assert_eq!(summary.sync_percent(), 100.0);
}
