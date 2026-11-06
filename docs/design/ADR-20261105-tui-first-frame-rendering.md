---
type: ADR
title: ADR-20261105-tui-first-frame-rendering：Windows 控制台首帧渲染缺陷的规避
description: Windows Terminal / ConPTY 在 alternate screen 切换后首帧布局错位的一整套缓解措施——缓冲区同步、尺寸稳定轮询、强制重绘、真实 resize 触发
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-tui-first-frame-rendering：Windows 控制台首帧渲染缺陷的规避

> English summary: On Windows Terminal / conhost + ConPTY, the first frame
> after the alternate-screen switch is frequently mislaid (bottom-row tear,
> status bar interleaved with content) even though a manual resize always
> fixes it. `run_tui`/`run_app` apply a layered mitigation: sync the console
> buffer to the visible window (`sync_console_buffer_size`), poll the size
> until it is stable for 10 consecutive 50ms reads (max 3s) using the WinAPI
> `srWindow` rectangle instead of `crossterm::terminal::size()` (which can
> return the buffer size or a stale size), draw + flush one frame under
> `BeginSynchronizedUpdate`, redraw twice more with 80ms gaps, end the
> synchronized update, and finally force one real terminal resize (shrink by
> 1 column, restore) which makes ConPTY reallocate and re-layout — the
> last-resort workaround that empirically removes the corruption. Mouse
> capture stays disabled because it misbehaves on Windows terminals.

## 问题

Windows 终端栈（Windows Terminal、conhost + ConPTY）在 alternate screen
切换后的首帧有多重不同步：

1. 控制台屏幕缓冲区可能大于可见窗口 → 出现滚动条，ratatui 把内容画到
   可见区域之外；
2. `crossterm::terminal::size()` 在首帧可能返回缓冲区大小或旧尺寸
   （`srWindow` 才是用户真实看到的矩形）；
3. PowerShell / cmd 在 alternate screen 切换瞬间写入的残留内容污染首帧；
4. 部分环境下首帧底行撕裂/错位无法靠纯软件重绘修复，但用户手动
   resize 后必然正常。

## 决定（按执行顺序，`cmd/syncthing/src/tui/runner.rs`）

1. 进入 alternate screen 前 `sync_console_buffer_size()`：用 WinAPI 把
   缓冲区设为 `srWindow` 大小，消除滚动条。
2. 进入后 sleep 100ms，给终端完成切换与尺寸同步。
3. `run_app` 首帧前尺寸稳定轮询：每次读 `get_current_console_size()`
   （WinAPI `srWindow` 优先，回退 crossterm），连续 10 次一致（约 500ms）
   或最多 60 次（3s）后认为窗口管理器完成最终布局。
4. 首帧强制序列：`autoresize` → `clear`（清掉切换瞬间的 shell 残留）→
   `hide_cursor` → `draw` + `flush`。
5. 再连续绘制 2 帧（间隔 80ms）并 flush，吸收终端后端缓冲区的同步延迟。
6. `EndSynchronizedUpdate`：此前 `BeginSynchronizedUpdate` 让终端延迟
   渲染直到完整帧就绪；现在允许正常刷新。直接对 stdout 下发，因为
   `run_app` 的 backend 类型参数不一定实现 `Write`。
7. `trigger_startup_resize()`：把终端宽度缩小 1 列再恢复（各等 50ms），
   欺骗终端完成一次真实缓冲区重排——终极兜底，消除残留的底行撕裂。
8. resize 后再清屏并绘制一帧，让 ratatui 内部缓冲与新终端状态同步。

此外：panic hook 在任何 panic 路径上恢复 raw mode / alternate screen；
鼠标捕获保持禁用（Windows 终端行为异常）。

## 理由

前 6 步覆盖了尺寸不同步、残留内容、渲染时机三类成因；第 7 步利用
"resize 后一定正常"的终端行为，把不可修复的首帧撕裂变成可编程触发
的恢复路径。各步骤独立可验证，组合后实测消除首帧布局错位。
