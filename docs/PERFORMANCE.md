# Performance and footprint

The native rewrite removes the bundled .NET runtime and Node.js MCP host. Migration measurements
on the same Windows desktop recorded **about 93% lower private memory for the resident HUD**.
The complete 4.0.0 release payload is **about 98% smaller** than the recorded 3.3.0 installation.

| Metric | 3.3.0 | 4.0.0 | Change |
| --- | --- | --- | --- |
| Resident HUD private memory | ≈ 176 MiB | ≈ 12–13 MiB | **≈ 93% lower** |
| Resident HUD working set | ≈ 177 MiB | ≈ 30 MiB | **≈ 83% lower** |
| Application/runtime payload | ≈ 178 MB installed | 3.77 MB unpacked release, including documentation and screenshots | **≈ 98% smaller** |
| Recorded idle CPU | — | ≈ 0.2% of one logical core | |

## Measurement scope

- The 3.3.0 process measurements were recorded on **2026-10-24**; the native release-build
  process measurements were recorded on **2026-10-25**, on the same Windows desktop with live
  agent sessions. These are migration observations rather than a cross-machine benchmark.
- Memory rows describe the **resident HUD process**, excluding MCP helpers and the agent
  applications themselves. Private memory and working set are different metrics; do not add them.
- The CPU figure is expressed relative to one logical core, rather than the whole-machine CPU
  percentage displayed by some tools.
- File sizes were checked against the published **v4.0.0** archive: **1,492,427 bytes** downloaded,
  **3,768,912 bytes** unpacked, and **2,382,336 bytes** for `AgentMonitorHUD.exe`. MB means
  1,000,000 bytes; memory MiB means 1,048,576 bytes.
- Reductions are rounded: private memory is about 92.6–93.2% lower across the recorded range,
  working set about 83.1% lower, and the full payload about 97.9% smaller.

The release keeps the complete native settings window, task list, independent bubbles, themes,
Claude hooks and MCP mode. These results do not depend on replacing the interface with a minimal
demonstration window.

## 中文

原生重写后的已有同机记录显示：**HUD 常驻私有内存约降低 93%，工作集约降低 83%**。
完整发布包解压后约 **3.77 MB**，包含说明文档和截图；相较旧版约 178 MB 的安装内容，
体积约缩小 **98%**。其中核心 EXE 为 **2.38 MB**，ZIP 下载体积为 **1.49 MB**。

内存和 CPU 来自 2026-10-24 至 25 日迁移期间的桌面实测记录；体积取自已发布的 v4.0.0
安装包。内存表仅统计 HUD 常驻进程，不包含 MCP 辅助进程和 Codex、Claude 等上游应用。
私有内存与工作集分别列出，不相加；CPU 按单个逻辑核心计。实际占用会随任务数量、视图和动画变化。
