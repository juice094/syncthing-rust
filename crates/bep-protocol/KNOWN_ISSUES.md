# Known Issues - bep-protocol

> ⚠️ **STATUS: UNVERIFIED - Agent-A Delivery**
> 
> This document tracks known issues and limitations in the BEP protocol implementation.
> All code in this crate is preliminary and has NOT been verified against actual Syncthing devices.

## UNVERIFIED Implementation Status

### General Status
- [ ] Code has NOT been tested against real Syncthing devices
- [ ] Protocol compliance has NOT been verified
- [ ] TLS certificate handling needs review
- [ ] Message encoding/decoding needs validation
- [ ] Error handling may be incomplete

### Module Status

#### messages.rs
- **Status**: ⚠️ UNVERIFIED
- **Issues**:
  - Wire format may not match official Syncthing BEP
  - XDR encoding/decoding of Hello messages needs validation
  - protobuf-like encoding is custom, may have compatibility issues
  - Field ordering and types need verification against BEP spec

#### codec.rs
- **Status**: ⚠️ UNVERIFIED
- **Issues**:
  - Custom binary encoding instead of actual protobuf
  - No compression support (LZ4 not implemented)
  - Message framing may not match BEP spec
  - No handling for message pipelining

#### handshake.rs
- **Status**: ⚠️ UNVERIFIED
- **Issues**:
  - Certificate verification is permissive (accepts any cert)
  - Device ID extraction from certificate needs validation
  - TLS configuration may need hardening
  - No support for custom CA certificates
  - Uses `dangerous()` API for client connections

#### connection.rs
- **Status**: ⚠️ UNVERIFIED
- **Issues**:
  - Request/response matching is simplified and may not work correctly
  - No proper handling for concurrent block requests
  - Message routing between sender and receiver needs review
  - Ping/pong keepalive not implemented
  - Connection state management is basic
  - Hello message exchange not integrated

## Missing Features

### Required for BEP v1 Compliance
- [ ] **LZ4 Compression** - Messages should support LZ4 compression
- [ ] **Message Pipelining** - Allow multiple in-flight requests
- [ ] **Proper Request Matching** - Match responses to requests correctly
- [ ] **Ping/Pong Keepalive** - Connection keepalive mechanism
- [ ] **ClusterConfig Handling** - Full cluster config exchange
- [ ] **Protocol Version Negotiation** - Verify BEP version compatibility

### Performance Optimizations
- [ ] **Zero-copy decoding** - Reduce memory copies
- [ ] **Buffered I/O** - Optimize read/write patterns
- [ ] **Connection pooling** - Reuse connections where possible

### Security Hardening
- [ ] **Certificate pinning** - Verify known devices
- [ ] **Rate limiting** - Prevent DoS attacks
- [ ] **Message size limits** - Enforce stricter limits

## Known Bugs

### None documented yet
- Bugs will be documented here as they are discovered during testing

## Testing Status

### Unit Tests
- [x] Basic encode/decode roundtrip tests
- [x] Hello message tests
- [x] TLS handshake tests
- [ ] Integration tests (missing)
- [ ] Compatibility tests with Syncthing (missing)

### Integration Tests Needed
1. Connect to real Syncthing instance
2. Perform full index exchange
3. Request and receive blocks
4. Handle concurrent requests
5. Test reconnection scenarios

## Compatibility Notes

### BEP Version
- Target: BEP v1 (as used by Syncthing)
- Implementation: Custom approximation

### TLS Requirements
- Minimum: TLS 1.2
- Client certificates required
- No server certificate verification (BEP design)

## TODO List

### High Priority
1. [ ] Implement proper protobuf encoding (use `prost`)
2. [ ] Add LZ4 compression support
3. [ ] Fix request/response matching
4. [ ] Add integration tests with real Syncthing
5. [ ] Verify protocol compliance

### Medium Priority
1. [ ] Add message pipelining
2. [ ] Implement keepalive (ping/pong)
3. [ ] Add proper error recovery
4. [ ] Improve logging and diagnostics

### Low Priority
1. [ ] Performance optimizations
2. [ ] Additional metrics
3. [ ] Documentation improvements

## References

- [BEP Protocol Specification](https://docs.syncthing.net/specs/bep-v1.html)
- [Syncthing Documentation](https://docs.syncthing.net/)

## Contact

- **Maintainer**: Agent-A
- **Status**: UNVERIFIED
- **Last Updated**: 2024
