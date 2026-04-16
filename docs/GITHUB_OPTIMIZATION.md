# GitHub 页面优化指南

> 以下内容供手动配置 `https://github.com/juice094` 个人主页及四个仓库使用。

---

## 一、个人资料页（Profile）

### 1. 创建 Profile README
新建仓库：`juice094/juice094`（与用户名完全一致），在仓库根目录放置 `README.md`：

```markdown
# 酒宿 / juice

Building local-first, privacy-preserving infrastructure in Rust.

## Projects

| Project | What it does | Stack |
|---------|-------------|-------|
| [**syncthing-rust**](https://github.com/juice094/syncthing-rust) | Rust implementation of the Syncthing protocol (BEP) with real-world Go interop | Rust, tokio, rustls, axum |
| [**clarity**](https://github.com/juice094/clarity) | Local-first AI Agent framework with TUI and MCP protocol support | Rust, ratatui, axum |
| [**devbase**](https://github.com/juice094/devbase) | Developer workspace database and knowledge-base manager | Rust, SQLite, git2 |
| [**agri-paper**](https://github.com/juice094/agri-paper) | Agricultural disease knowledge base and LLM evaluation framework | Python, LaTeX |

> Currently focused on making **syncthing-rust** a production-viable P2P sync daemon.
```

### 2. 个人资料 Bio 设置
在 GitHub Settings → Profile 中设置：
- **Bio**: `Rust · Local-first · P2P sync · AI infrastructure`
- **Company/Socials**（可选）：个人博客或 Twitter 链接

---

## 二、仓库描述与 Topics

在 GitHub 仓库页面点击右侧齿轮（⚙️）设置以下信息：

### syncthing-rust
```
Description: A Rust implementation of the Syncthing protocol stack with BEP interoperability
Topics: rust, syncthing, p2p, file-sync, bep, tls, distributed-systems
Website: （可选，后续可放文档站点）
```

### clarity
```
Description: Local-first AI Agent framework in Rust with TUI and MCP support
Topics: rust, ai-agent, llm, mcp, tui, local-first, ratatui
```

### devbase
```
Description: Developer workspace database and knowledge-base manager
Topics: rust, developer-tools, knowledge-base, sqlite, git
```

### agri-paper
```
Description: Agricultural disease knowledge base and LLM evaluation framework
Topics: agriculture, knowledge-base, llm, crop-disease, ai-agriculture
```

---

## 三、仓库功能开关建议

### 建议开启
- **Issues** — 用于 bug 追踪和功能请求
- **Discussions** — 用于问答和想法交流（clarity 和 syncthing-rust 优先开启）

### 建议关闭（当前阶段）
- **Wiki** — 容易分散文档注意力，统一用 `docs/` 目录管理
- **Projects** — 除非 actively 使用 GitHub Projects 看板，否则空白项目板会降低页面观感

---

## 四、README 视觉增强（未来可选）

当 CI 建立后，可在 `syncthing-rust/README.md` 顶部追加以下 badges：

```markdown
[![CI](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/juice094/syncthing-rust/actions)
[![crates.io](https://img.shields.io/crates/v/syncthing)](https://crates.io/crates/syncthing)
```

（当前尚未配置 GitHub Actions，可先预留位置。）
