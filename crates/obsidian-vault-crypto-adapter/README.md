# Obsidian Vault Crypto Adapter

为 Obsidian vault 提供端到端加密，使 syncthing-rust 只同步密文。

## 背景

syncthing-rust 自身只做 TLS 传输加密，没有应用层端到端加密。其 `AGENTS.md` 将「自定义加密」列为冻结项，因此不能直接在 syncthing-rust 内实现 E2EE。

本适配器采用 **Path A**：在 syncthing-rust 外部加一层加密/解密，让它只看见密文。

## 加密方案

- **密钥派生**：`Argon2id(password, salt)` → master key → HKDF-SHA256 分出 content key 与 filename key
- **文件名加密**：AES-256-SIV（确定型，同名文件始终映射为同一名称）
- **文件内容加密**：AES-256-GCM，每个文件独立随机 nonce
- **密钥清零**：使用 `zeroize`，内存中不残留明文密钥

## 用法

```bash
# 1. 生成 salt（每个 vault 一个，可公开存储）
cargo run --release -- gen-salt --output vault.salt

# 2. 把明文 vault 加密到 sync-folder
cargo run --release -- encrypt \
    --vault ~/Obsidian/MyVault \
    --sync ~/Sync/MyVault-encrypted \
    --password "your-strong-password" \
    --salt-file vault.salt

# 3. 让 syncthing-rust 同步 ~/Sync/MyVault-encrypted

# 4. 在另一端解密回明文 vault
cargo run --release -- decrypt \
    --sync ~/Sync/MyVault-encrypted \
    --vault ~/Obsidian/MyVault-restored \
    --password "your-strong-password" \
    --salt-file vault.salt
```

## 当前状态

- ✅ 文件名单向/往返加解密
- ✅ 文件内容单向/往返加解密
- ✅ 目录级加密/解密（保持目录结构）
- ✅ 篡改检测（GCM tag 校验失败）
- ✅ 错误密码拒绝解密
- ✅ CLI：`gen-salt` / `encrypt` / `decrypt`
- ✅ 与 syncthing-rust 端到端联调通过（`syncthing-rust/cmd/syncthing/tests/e2e_encrypted_vault_sync.rs`）

## 局限与后续

- **文件级加密**：改动一个字符会重传整个文件（未来可补块级加密或内容寻址去重）
- **全量扫描**：当前每次 `encrypt`/`decrypt` 都遍历整树（后续加 watcher/增量）
- **忽略隐藏文件**：默认跳过 `.git`、`.DS_Store` 等，但 `.obsidian` 保留
- **移动端集成**：Phase 2 将适配器 + syncthing-rust 封装为 Android 后台服务

## 项目位置

`C:/Users/22414/dev/obsidian-vault-crypto-adapter/`

相关项目：
- `C:/Users/22414/dev/syncthing-rust/` — 传输层
- `C:/Users/22414/dev/obsidian-secure-storage-rust/` — Android Keystore 安全存储（可用来存 salt 或密码）
