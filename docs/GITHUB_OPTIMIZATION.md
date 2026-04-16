# GitHub 页面优化指南 — syncthing-rust

> 以下内容仅针对 `syncthing-rust` 仓库的 GitHub 页面优化。

---

## 一、仓库描述与 Topics

在仓库页面右侧齿轮（⚙️）中设置：

```
Description: A Rust implementation of the Syncthing protocol stack with BEP interoperability
Topics: rust, syncthing, p2p, file-sync, bep, tls, distributed-systems
Website: （可选，后续可放文档站点）
```

---

## 二、仓库功能开关建议

### 建议开启
- **Issues** — 用于 bug 追踪和功能请求
- **Discussions** — 用于问答和社区交流

### 建议关闭（当前阶段）
- **Wiki** — 文档统一由 `docs/` 目录管理，避免分散
- **Projects** — 除非 actively 使用 GitHub Projects 看板，否则空白项目板会降低页面观感

---

## 三、README 视觉增强（未来可选）

当 CI 建立后，可在 `README.md` 顶部追加以下 badges：

```markdown
[![CI](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/juice094/syncthing-rust/actions)
[![crates.io](https://img.shields.io/crates/v/syncthing)](https://crates.io/crates/syncthing)
```

（当前尚未配置 GitHub Actions，可先预留位置。）
