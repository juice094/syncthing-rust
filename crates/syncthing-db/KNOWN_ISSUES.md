# Known Issues - syncthing-db (Agent-E Delivery)

> ⚠️ **WARNING**: This implementation is **UNVERIFIED** and has not been validated by the Master Agent.
> DO NOT USE IN PRODUCTION.

## Implementation Limitations

### 1. Cache Eviction Strategy

**Issue**: The LRU cache implementation uses a simple timestamp-based eviction strategy.

**Impact**: Under heavy concurrent access, the eviction order may not perfectly reflect true LRU semantics.

**Workaround**: Cache size is configurable; reduce cache size if memory pressure is observed.

### 2. Hash Verification on Read

**Issue**: Block hashes are verified on write but not on read.

**Impact**: Data corruption in underlying storage may go undetected on read operations.

**Mitigation**: Consider adding optional hash verification on read for critical data paths.

### 3. Metadata Serialization

**Issue**: File metadata is serialized using JSON.

**Impact**: 
- JSON serialization is slower than binary formats like bincode or Protocol Buffers
- Larger storage footprint compared to binary formats
- May not preserve all SystemTime precision across all platforms

**Future Improvement**: Migrate to a binary serialization format for better performance.

### 4. Sled Limitations

**Issue**: The implementation relies on sled 0.34 which has known limitations.

**Known sled issues**:
- Database file size may grow over time due to append-only design
- No built-in compaction (requires manual `sled::Db::flush` and periodic cleanup)
- Windows file locking may cause issues with database reopening

**Mitigation**: Regular maintenance (flush operations) and monitoring of database size.

### 5. Error Handling Granularity

**Issue**: All sled errors are mapped to generic `SyncthingError::Storage`.

**Impact**: Difficult to distinguish between different failure modes (e.g., disk full vs. corruption).

**Future Improvement**: Add more specific error variants to `SyncthingError`.

### 6. Concurrent Write Ordering

**Issue**: The current implementation does not guarantee strict ordering of concurrent writes.

**Impact**: In rare cases, concurrent updates to the same file metadata may result in last-write-wins behavior.

**Mitigation**: Applications should implement higher-level coordination if strict ordering is required.

### 7. Cache Consistency on Delete

**Issue**: Block deletion removes entries from cache immediately but the sled delete is not atomic with cache removal.

**Impact**: A crash between sled delete and cache removal could leave stale cache entries on restart.

**Mitigation**: This is a minor issue as cache is in-memory only and will be empty on restart.

### 8. Missing Features

The following features are NOT implemented:

- [ ] Compression for stored blocks
- [ ] Encryption at rest
- [ ] Block reference counting (for garbage collection)
- [ ] Incremental backup support
- [ ] Database migration/versioning
- [ ] Metrics and monitoring hooks
- [ ] Configurable flush intervals

### 9. Testing Coverage

**Current State**: Basic unit tests are implemented but lack:

- [ ] Property-based testing for hash correctness
- [ ] Long-running stress tests
- [ ] Disk-full error scenarios
- [ ] Corruption recovery tests
- [ ] Cross-platform testing (Windows, macOS, Linux)

## Integration Notes

### With syncthing-core

This implementation correctly implements the `BlockStore` trait from `syncthing-core`, but note:

- `FolderStats` is calculated on-demand and may be expensive for large folders
- Index operations are synchronous with sled operations (blocking tokio threads)

### Performance Characteristics

| Operation | Expected Latency | Notes |
|-----------|-----------------|-------|
| Block get (cache hit) | < 1μs | In-memory lookup |
| Block get (cache miss) | 1-10ms | Disk read |
| Block put | 1-10ms | Write + optional flush |
| Index get | 10-100ms | Scans all folder entries |
| Index update | 10-100ms | Batch operation |

## Security Considerations

⚠️ **IMPORTANT**:

1. No encryption of data at rest - database files are stored in plaintext
2. No verification of block hashes on read (trusts underlying storage)
3. Cache may retain sensitive data in memory longer than necessary

## Recommendations for Production Use

Before using in production:

1. Implement comprehensive backup strategy
2. Add monitoring for database size and cache hit rates
3. Perform load testing with realistic data volumes
4. Consider implementing compression for large blocks
5. Add periodic database compaction
6. Implement proper error recovery procedures

## Feedback and Issues

Report issues to the Master Agent for review before any production deployment.

---

**Last Updated**: 2024 (Agent-E Delivery)
**Status**: UNVERIFIED
