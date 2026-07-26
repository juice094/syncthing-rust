# Obsidian Vault Crypto Adapter

## 目标

在 **不改变 syncthing-rust 源码** 的前提下，为 Obsidian vault 提供端到端加密同步能力。

- syncthing-rust 只同步**密文**
- 服务端 / relay / 中间节点无法读取笔记内容
- 对端设备需要密码/密钥才能解密

## 架构位置

```
Obsidian vault（明文）
    ↑↓
obsidian-vault-crypto-adapter（本适配器）
    ↑↓
sync-folder（密文）
    ↑↓
syncthing-rust（P2P 同步）
    ↑↓
对端 sync-folder（密文）
    ↑↓
对端 obsidian-vault-crypto-adapter
    ↑↓
对端 Obsidian vault（明文）
```

## 加密方案

### 密钥派生

```
master_key = Argon2id(password, salt, m=64MB, t=3, p=4)
content_key = HKDF-SHA256(master_key, salt="", info="obsidian-vault-content-v1")
filename_key = HKDF-SHA256(master_key, salt="", info="obsidian-vault-filename-v1")
```

### 文件名加密

使用 **AES-256-SIV**（确定型 AEAD），相同文件名总是产生相同密文：

```
encrypted_name = AES-256-SIV.encrypt(filename_key, utf8(filename))
stored_name = base32hex(encrypted_name)  // 文件系统安全
```

这样无需维护文件名映射表，也避免映射文件冲突。

### 文件内容加密

使用 **AES-256-GCM**：

```
nonce = 96-bit random
encrypted = AES-256-GCM.encrypt(content_key, plaintext)
stored = version || nonce || ciphertext || tag
```

存储格式（字节）：

| 偏移 | 长度 | 内容 |
|---|---|---|
| 0 | 1 | 版本（当前 0x01）|
| 1 | 12 | nonce |
| 13 | N | ciphertext + tag |

### 目录结构

加密后的目录保持与原 vault 相同的树形结构，只是文件名被替换。

```
sync-folder/
├── AB32X...            # 原 "Daily Notes.md"
├── CD91Y.../           # 原 "Projects/"
│   └── EF12Z...        # 原 "Projects/Idea.md"
└── ._obsidian_config   # 原 ".obsidian/" 配置目录
```

## 适配器职责

### Phase 1 PoC（已完成）

- `encrypt-dir`：把明文 vault 完整加密到 sync-folder
- `decrypt-dir`：把 sync-folder 完整解密回明文 vault
- 验证 round-trip、文件名确定性、篡改检测
- 目录级单元测试 5/5 通过

### Phase 2（已完成）

- ✅ 与 syncthing-rust 联动验证：加密 vault 经 BEP 同步到对端，解密后原文一致
- 集成测试：`syncthing-rust/cmd/syncthing/tests/e2e_encrypted_vault_sync.rs`（1 passed）
- 待补：双向同步、增量扫描/watcher

### Phase 3（后续）

- Android 服务化
- Capacitor 插件
- Obsidian APK 集成

## 安全边界

- 密码只在适配器内存中存在，用 `zeroize` 清零
- 不在磁盘上保存明文密钥
- salt 可公开存储在 sync-folder 根目录（`salt.bin`）
- 不提供密码找回：密码丢失 = 数据永久不可解密

## 依赖

- `aes-gcm`：AES-256-GCM
- `aes-siv`：AES-256-SIV（确定型文件名加密）
- `hkdf`：密钥派生
- `argon2`：密码派生
- `zeroize`：密钥清零
- `base32`：文件系统安全编码
- `walkdir`：目录遍历
- `clap`：CLI

## 非目标

- 不实现块级加密（先文件级，够用即可）
- 不实现文件历史版本（交给 syncthing-rust 的 versioner）
- 不实现 Obsidian 语义同步（Phase 3 再考虑）
