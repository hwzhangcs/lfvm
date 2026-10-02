# LFVM — 基于 Tauri 的本地文件版本管理与可视化系统

《软件工程方法与实践》第三组（周瀚文、戴烨铭、张瀚文、邓皓云）。需求依据：《系统需求规格说明书》V2.1（LFVM-SRS-0001）。

## 目录结构

```
crates/lfvm-platform/   平台差异层：链接/云占位识别、原子替换、大小写探测、文件名规则（Windows/macOS/Linux）
crates/lfvm-core/       业务核心，不依赖 Tauri；全部功能与测试都在这里
  migrations/           SQLite 结构（对应 SRS 第 4 章）
apps/desktop/           桌面程序
  src-tauri/            Rust 外壳：界面命令、对话框、窗口
  src/                  React + Ant Design 前端
    bindings.ts         由 Rust 自动生成的命令与类型，勿手改
    locales/zh-CN.ts    全部界面文字（用语检查见 terms.test.ts）
.github/workflows/      三平台 CI；打 v* 标签自动出安装包
```

## 开发环境

- Rust stable、Node 24、pnpm
- Linux 需要：`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libssl-dev`
- macOS 需要：`xcode-select --install`；Windows 需要：WebView2（Win10/11 自带）与 MSVC 生成工具

## 常用命令

```bash
# 核心库测试（最常用，不需要界面）
cargo test -p lfvm-core -p lfvm-platform

# 更新前端类型 bindings.ts（修改了命令或对外数据结构后执行）
cargo test -p lfvm-desktop

# 启动桌面程序（开发模式，前端热更新）
cd apps/desktop && pnpm install && pnpm tauri dev

# 前端检查
cd apps/desktop && pnpm lint && pnpm typecheck && pnpm test

# 打安装包（当前平台）
cd apps/desktop && pnpm tauri build
```

## 约定

- 业务逻辑只写在 `lfvm-core`；`src-tauri/src/commands.rs` 只做参数转换。
- 前端只调用 `bindings.ts` 中的固定命令，不传任意路径（LFVM-IF-05）。
- 界面文字只写在 `locales/zh-CN.ts`，不得出现 commit、branch、HEAD 等术语（表 8-2）。
- 错误统一为 `CoreError { code, message, path }`，新增错误码只在末尾追加。

## 里程碑

| 阶段 | 内容 | SRS |
|---|---|---|
| **M0** ✅ | 工程骨架、平台层、数据库结构、错误码、界面框架、CI | — |
| **M1** ✅ | 加入/打开/移除项目、排除项、扫描、内容对象库、保存版本 | 1.1–1.3 |
| **M2** ✅ | 时间地图、节点详情、历史文件树、预览、版本比较 | 2.1–2.3 |
| **M3** ✅ | 操作引擎：安全备份、单文件恢复、整版恢复、崩溃恢复、备份找回 | 2.4、2.5、3.1–3.3 |
| **M4** ✅ | 方案：创建、改名、切换，多方案时间地图 | 4.1、4.2 |
| **M5** ✅ | 搜索历史文件、缩略图、文件轨迹 | 3.4、3.5 |
| M6 | 展开、导出、存储占用与清理 | 4.3、4.4、5.1 |
| M7 | 性能与稳定性测试、可用性测试、安装包 | 第 5、6 章 |
