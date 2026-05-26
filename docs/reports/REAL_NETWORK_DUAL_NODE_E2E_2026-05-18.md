---
type: report
status: completed
project: syncthing-rust
date: 2026-05-18
tags: [report, testing, e2e, network]
---

# Real-Network Dual-Node E2E Test Report

> **Date:** 2026-05-18
> **Version:** v0.2.9-rc2
> **Topology:** Windows 11 (local, Tailscale 100.107.247.38) ↔ Ubuntu 24.04 (remote, Tailscale 100.127.13.26)
> **Network:** Tailscale virtual overlay (campus firewall bypass)

---

## 1. Test Objective

Validate bidirectional file sync between Windows and remote Linux syncthing-rust nodes over real Tailscale network, verifying:
- BEP handshake over Tailscale tunnel
- Persistent TCP connection across NAT/firewall
- Block-level file transfer
- API state consistency

---

## 2. Environment

| Component | Node-A (Windows Local) | Node-B (Ubuntu Remote) |
|-----------|------------------------|------------------------|
| OS | Windows 11 Pro (26200) | Ubuntu 24.04 LTS (6.8.0-55) |
| Binary | syncthing.exe (debug) | syncthing (ELF, deploy-remote) |
| Tailscale IP | 100.107.247.38 | 100.127.13.26 |
| Listen | 0.0.0.0:22001 | 0.0.0.0:22001 |
| API | 127.0.0.1:8385 | 0.0.0.0:8385 |
| Sync Dir | `C:\Users\22414\syncthing-real-test\node-a\sync` | `/tmp/syncthing-test-node/sync` |
| Device ID | 3ODMWTB-FEFA7UF-B2DTX7T-NNSKJLE-5V722Y7-FOTE36C-FEYAGOJ-SAKLKQI | IBZ76D5-SGKBFSB-Z572ZOY-BMP3EOO-ZNKQEIS-KUFXYMW-EYZGXLE-OKLX4AJ |

---

## 3. Deployment Notes

- **SSH Block Resolved**: Gray-Cloud configured ed25519 pubkey auth on remote server.
- **SCP Transfer**: `syncthing` (13.6MB) + `syncthing-monitor` (2.9MB) deployed to `/tmp/syncthing-test-node/`.
- **Old Process Cleanup**: Pre-existing `syncthing-v0.2.` (PID 1234108) killed before new deployment.
- **Config Generation**: Both nodes auto-generated certificates; configs updated with mutual peer addresses via Tailscale IPs.

---

## 4. Test Results

### 4.1 Connection Establishment

| Check | Expected | Actual | Status |
|-------|----------|--------|--------|
| TCP dial over Tailscale | Remote reachable on :22001 | `connected: true` | ✅ |
| TLS 1.3 handshake | Certificate validated | `type: tcp-server` | ✅ |
| BEP Hello/ClusterConfig | Folder shared | Shared `real-test` folder | ✅ |

**API Evidence (local node):**
```json
{
  "connections": {
    "IBZ76D5-SGKBFSB-Z572ZOY-BMP3EOO-ZNKQEIS-KUFXYMW-EYZGXLE-OKLX4AJ": {
      "connected": true,
      "type": "tcp-server",
      "address": "100.127.13.26:22001"
    }
  }
}
```

### 4.2 File Sync (Local → Remote)

| Check | Expected | Actual | Status |
|-------|----------|--------|--------|
| Local file creation | Watcher detects change | `files: 1, bytes: 31` | ✅ |
| Index propagation | Remote receives index | `globalBytes: 31` | ✅ |
| Block transfer | Remote requests blocks | API state `idle` | ✅ |
| Remote consistency | `globalBytes == localBytes` | `31 == 31` | ✅ |

**API Evidence:**
```json
{
  "folder": "real-test",
  "files": 1,
  "bytes": 31,
  "need_files": 0,
  "need_bytes": 0,
  "globalBytes": 31,
  "localBytes": 31,
  "state": "idle"
}
```

### 4.3 File Sync (Remote → Local)

| Check | Expected | Actual | Status |
|-------|----------|--------|--------|
| Remote file creation | SSH into remote, create file | `test_rust_b.txt` created | ✅ |
| Local reception | File appears in local sync dir | Received in ~10s | ✅ |

**API Evidence:**
```json
{
  "folder": "real-test",
  "files": 7,
  "bytes": 183,
  "need_files": 0,
  "need_bytes": 0,
  "globalBytes": 183,
  "localBytes": 183,
  "state": "idle"
}
```

### 4.4 Large File Sync (512KB)

| Check | Expected | Actual | Status |
|-------|----------|--------|--------|
| 512KB file creation | Local generates random binary | `bigdata.bin` 524,288 B | ✅ |
| Index propagation | Remote receives index | `globalBytes == localBytes` | ✅ |
| Block transfer | Remote requests blocks | API state `idle` | ✅ |
| Remote consistency | `globalBytes == localBytes` | `524471 == 524471` | ✅ |

**注**：1MB+ 大文件首次测试因 `.syncthing.tmp` 冲突机制导致文件被清理；512KB 文件传输验证通过。

---

## 5. Issues Observed

| ID | Issue | Severity | Notes |
|----|-------|----------|-------|
| OBS-1 | SSH timeout during test | Medium | Tailscale BEP channel stable; SSH 控制平面独立，偶发超时。不影响同步验证结论。 |
| OBS-2 | Remote API (`:8385`) unreachable from local | Low | Remote binds to `0.0.0.0:8385`，但 Tailscale 防火墙或安全组可能拦截非 BEP 端口。 |
| OBS-3 | syncthing-monitor not started | Low | 仅部署了二进制，未启动 monitor 进程（本次聚焦 sync 验证）。 |

---

## 6. Conclusion

**Result: PASS** (Updated 2026-05-19)

真实网络 BEP 连接和双向文件同步已验证成功。Tailscale 虚拟网络有效绕过了校园网防火墙对 TCP 22001 的阻断，两端通过 `100.x.x.x` 地址稳定建立 TLS + BEP 会话，并完成块级文件传输。

**已验证**：
- Tailscale 穿透可行性 ✅
- BEP 握手 + ClusterConfig + Index ✅
- 文件同步（Windows → Linux 远程）✅
- 文件同步（Linux 远程 → Windows）✅
- 大文件（512KB）真实网络传输 ✅

**待补全**：
- 1MB+ 大文件真实网络传输 ⏳（512KB 通过，1MB 因 `.syncthing.tmp` 冲突需修复）
- 72h 耐久性 ⏳（Phase 2）

---

*Report generated: 2026-05-18*
