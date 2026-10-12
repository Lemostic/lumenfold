<div align="center">

# Lumenfold

**把光折叠起来：本地优先、可插件扩展的照片资产管理器**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](./LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-v2-FFC131?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.80%2B-orange?logo=rust)](https://www.rust-lang.org)
[![CI](https://img.shields.io/badge/CI-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)](./.github/workflows/ci.yml)

跨 Windows / macOS / Linux 的桌面照片库。照片留在你自己的硬盘上，索引、缩略图、识别全部本地完成，不上传任何一张图。

</div>

---

## 它是什么

一个**可编程的照片库**，而不是又一个看图软件。

- **本地优先**：文件永远在你指的地方，Lumenfold 只读不放，改动需显式确认。
- **插件是一等公民**：数据库查询、元数据处理、导出流水线、面板 UI 都能由第三方扩展，宿主不内置死功能。
- **全部能力离线**：人脸识别、语义搜索、自动标签走本地 ONNX 推理，模型可下载后完全断网运行。
- **扛得住大库**：目标 20 万张规模，冷启动到可交互 <6s，虚拟滚动不掉帧。

## 功能规划

| 能力 | 说明 | 里程碑 |
|---|---|---|
| RAW / HEIC 预览 | CR2/NEF/ARW/HEIC/AVIF，色彩管理，多尺寸缩略图 | M1 |
| 元数据编辑与导出 | XMP/IPTC 写回、批量重命名、条件导出归档 | M2 |
| 插件系统 | QuickJS 沙箱 + 能力网关，四层扩展点 | M3 |
| AI 语义检索 | 本地 CLIP，以文搜图 / 以图搜图 / 自动标签 | M4 |
| 人脸与人物相册 | 本地检测嵌入、聚类、手工合并命名 | M5 |
| 时间线 / 地图 / 相似聚类 | 按拍摄浏览、GPS 地图、去重与相似图归组 | M6 |

## 架构速览

```
WebView (React)  ──lumen://thumb/*──▶  Rust 宿主  ──▶  SQLite (WAL + FTS5 + vec)
       ▲                                 │
       │                          缩略图流水线 / 推理 Worker
       │                                 │
   插件 iframe  ◀──能力网关──▶  QuickJS 沙箱  ◀── 第三方插件
```

细节见 [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [docs/ROADMAP.md](./docs/ROADMAP.md) | 开发计划：里程碑、交付物、完成判据、性能预算、风险 |
| [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) | 分层架构、数据模型、插件沙箱设计 |
| [docs/UI-SPEC.md](./docs/UI-SPEC.md) | 界面规范：外壳布局、颜色与字体令牌、圆角、动效、可访问性 |
| [docs/adr/](./docs/adr/) | 架构决策记录（技术选型与许可证） |

## 开发

```bash
git clone https://github.com/lumenfold/lumenfold
cd lumenfold
pnpm install
pnpm tauri dev
```

`pnpm tauri dev` 会先起 Vite（`127.0.0.1:1420`，Tauri 固定连这个端口），再拉起桌面窗口。

只想看界面布局，不启动 Rust 宿主时：

```bash
pnpm dev
```

浏览器里没有宿主，界面会切到演示图库（固定种子的 1800 张占位数据，用于评估布局）。此时状态栏常驻「演示图库」标记，缩略图用 picsum 占位摄影。演示模式不下载模型权重，设置页会明确说明清单与校验值均为示例数据。

前置依赖：Rust 1.80+、Node 20+。图像流水线的平台原生依赖（libvips / libheif / libraw）尚未接入，接入后会在此补充。

### 打包

日常出安装包用这条命令：

```bash
pnpm build:installer
```

它依次做三件事：

1. **查前置条件** —— Node 版本、`cargo` 是否在 PATH 上、`tauri.conf.json` 里声明的图标是否真的存在。
   这一步是为了 fail fast：图标缺失如果留到打包阶段才暴露，用户已经白等了四分钟的 Rust 编译。
2. **构建** —— 转调 `pnpm tauri:build`，也就是 `tauri build`。
3. **只报这次的产物** —— 递归扫描 `apps/desktop/src-tauri/target/release/bundle/`，
   只认「构建开始之后新写出来」的安装包，逐个打印相对路径与体积。

按时间筛而不是直接列目录，是为了不把上一轮遗留的旧包当成本次的成果报出来。
一个都没筛到时脚本以非零码退出：静默交付一个陈旧安装包比报错更糟。

首次构建要编译整个 Rust 依赖树，release 模式下大约五分钟；有缓存之后通常一两分钟。
脚本不签名也不公证，那属于 M7。

需要绕过脚本直接调 Tauri 时用：

```bash
pnpm tauri build
```

产物在 `apps/desktop/src-tauri/target/release/bundle/`，具体格式由 `tauri.conf.json` 的 `bundle.targets` 决定。

Windows 上 Rust 走 MSVC 链接器，需要 Visual Studio Build Tools。系统需要装有 WebView2 运行时，多数 Windows 10/11 已预装。

### 测试

```bash
pnpm typecheck                                        # 前端类型检查
cd apps/desktop/src-tauri && cargo test --workspace   # 后端单元测试
```

## 许可

源码 **AGPL-3.0-or-later**。动态链接 LGPL-2.1+ 组件（libvips / libheif / libraw），发行包内附第三方声明与源码获取入口。

模型权重**不随安装包分发**，也不进本仓库。安装完成后由使用者自行在应用内下载，下载前展示该模型的完整许可并需显式接受，接受动作留痕。

不同模型的许可不同：默认注册表只收录宿主能替用户判断许可的权重（MIT / Apache-2.0 且无额外部署限制）；许可不清或非商用的权重（如 InsightFace 系）走用户自备通道，宿主提供路径配置与加载能力，但不提供下载入口，也不代为断言许可。依赖这类模型的功能会在界面上显示「非商用」提示。宿主自身的 AGPL-3.0-or-later 不覆盖模型权重，也不能使非商用权重变为可商用。逐项核查结果与分发规则见 [docs/adr/0002-model-distribution-and-licensing.md](./docs/adr/0002-model-distribution-and-licensing.md)。

## 状态

**早期开发中，尚未发布。** 里程碑进度见 [ROADMAP](./docs/ROADMAP.md)。