# Profiling Guide · syncthing-rust

> **目标**：建立可重复的性能采集流程。本文档不替代 criterion 基准（见 `TUNING_PLAN_2026-05-11.md` T-A1），它解决"如何看到热点在哪里"。
> **平台**：Windows 主力 + Linux 移植可选。

---

## 一、采集工具栈

| 工具 | 用途 | Windows 支持 | 备注 |
|------|------|--------------|------|
| **cargo-flamegraph** | CPU 火焰图 | ✅ via `perf` (WSL) / DTrace / blondie | Windows 原生用 blondie |
| **dhat-rs** | 堆分配热点 | ✅ | 编译期 feature 开启 |
| **cargo-criterion** | 基准对比 | ✅ | T-A1 标配 |
| **tokio-console** | tokio runtime 实时可视化 | ✅ | 需开 `tracing` feature |
| **perfprobe / VTune** | 系统级 CPU/IO | ✅ Windows | 厂商工具，可选 |

---

## 二、cargo-flamegraph

### Windows 原生（blondie backend）

```powershell
# 安装
cargo install flamegraph
cargo install blondie

# 采集（必须以管理员身份运行 PowerShell）
$env:CARGO_PROFILE_RELEASE_DEBUG = "true"
cargo flamegraph --bin stress_test --release -- --duration 5m --report stress.csv
# 输出：flamegraph.svg（浏览器打开）
```

### Linux / WSL（perf backend）

```bash
sudo apt install linux-tools-common linux-tools-generic
cargo install flamegraph

# 采集
sudo CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
  --bin stress_test --release -- --duration 5m
```

### 常见热点定位

| 看到火焰图顶部出现 | 对应代码位置 | 调优任务 |
|--------------------|--------------|----------|
| `sha2::Sha256::digest` 占 >30% | `syncthing-fs/src/scanner.rs:91-117` | T-B1 |
| `tokio::fs::File::read` 同步阻塞 | scanner / database | T-B / T-C |
| `serde_json::to_string_pretty` | `syncthing-sync/src/database.rs` | T-C2 |
| `rustls` 握手 >50ms | `syncthing-net/src/tls.rs` | 通常正常，TLS 1.3 一次握手 |

---

## 三、dhat-rs（堆分析）

### 启用

`Cargo.toml`（应用 crate）增加：

```toml
[features]
dhat-heap = ["dep:dhat"]

[dependencies]
dhat = { version = "0.3", optional = true }
```

`main.rs` 包裹：

```rust
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    // ... 业务代码
}
```

### 采集

```powershell
cargo run --release --features dhat-heap --bin stress_test -- --duration 1m
# 输出：dhat-heap.json
# 上传到 https://nnethercote.github.io/dh_view/dh_view.html 可视化
```

### 关注指标

- `t-gmax`：进程峰值堆内存 — 异常突增提示泄漏
- `t-end`：进程结束时堆 — 应接近 0（除全局静态）
- `t-end` ÷ `t-gmax` > 0.5：可能泄漏

---

## 四、tokio-console（实时 task 可视化）

### 启用

`Cargo.toml`：

```toml
[dependencies]
console-subscriber = "0.4"
```

`main.rs`：

```rust
console_subscriber::init();  // 在 fmt subscriber 之前
```

`Cargo.toml` 顶层：

```toml
[build]
rustflags = ["--cfg", "tokio_unstable"]
```

### 启动

```powershell
# Terminal 1
cargo run --release --bin syncthing -- run

# Terminal 2
cargo install tokio-console
tokio-console
```

可见：所有 spawn 的 task、忙/闲时长、被唤醒频率。检测"幽灵 task"（spawn 后没 join）。

---

## 五、采集场景与对应任务

| 场景 | 命令 | 期望发现 | 关联调优任务 |
|------|------|----------|---------------|
| 单文件 1 GB 哈希 | `cargo flamegraph --bin syncthing-bench -- large` | 80%+ 时间在 sha2 | T-B1 |
| 10K 小文件扫描 | `cargo flamegraph --bin syncthing-bench -- mixed` | walkdir + 顺序 await 占大头 | T-B2 |
| BEP 握手 + 5 分钟同步 | `cargo flamegraph --bin stress_test -- --duration 5m` | TLS 握手一次性，其余为消息分发 | T-D2 |
| 长跑 1 小时堆 | `dhat-heap.json` from stress_test | RSS 是否单调上升 | T-C3 / T-F1 |

---

## 六、一键采集脚本

详见 [`scripts/profile.ps1`](../../scripts/profile.ps1)。

```powershell
# 5 分钟 CPU 火焰图
.\scripts\profile.ps1 -Mode cpu -Duration 5m

# 1 小时堆采样
.\scripts\profile.ps1 -Mode heap -Duration 1h

# tokio task 实时
.\scripts\profile.ps1 -Mode tasks
```

---

## 七、注意事项

- **release 必须开 debug=true**：profile.bench 已配置（`.cargo/config.toml`）；release-thin 手动加 `CARGO_PROFILE_RELEASE_DEBUG=true`
- **Windows blondie 需要管理员**：常规账户运行会缺少符号
- **WSL 中跑 syncthing-rust 时网络层差异**：UDP 多播、TCP keepalive 行为与 Windows 原生不同；仅做 CPU/堆采集时可用 WSL，网络相关 profile 必须 Windows 原生
- **首次运行 cargo flamegraph 需 30~60s 编译开销**：不要中断

---

## 八、相关文档

- [`docs/plans/TUNING_PLAN_2026-05-11.md`](../plans/TUNING_PLAN_2026-05-11.md) — T-A1/T-A2 任务定义
- [`AGENTS.md`](../../AGENTS.md) §代码健康 — 性能改动必须可量化
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — bench workflow
