# ClawPal Release / Prerelease 流程说明

本文基于当前仓库 `.github/workflows/bump-version.yml` 与 `.github/workflows/release.yml`（2026-03-04）整理，说明 `release` 与 `prerelease` 的实际执行流程，以及 Apple Developer 签名/公证行为。

## 1. 触发入口（推荐）

推荐通过 `Bump Version` workflow（手动触发）作为统一入口：

1. 校验目标版本（严格 semver + tag 冲突检查）
2. 更新代码版本（`package.json` / `src-tauri/Cargo.toml` / `src-tauri/Cargo.lock`）
3. 运行测试 CI（前端 typecheck/build + Rust fmt/clippy/test）
4. 运行打包 CI（4 平台矩阵，验证可打包）
5. 全部通过后才执行 commit + push（不打 tag）
6. `Bump Version` 直接 dispatch `Release` workflow 创建/更新 Draft Release
7. 人工审核后点击 Publish，GitHub 才会创建 `vX.Y.Z` tag

## 2. Release Workflow 触发条件

- Workflow: `Release`
- 触发事件: `workflow_dispatch`（由 `Bump Version` 触发）
- 输入:
  - `version`
  - `target_commitish`
  - `is_prerelease`
- 示例:
  - 正式版: `v0.1.1`
  - 预发布: `v0.1.1-beta.1` / `v0.1.1-rc.1`

## 3. 总体结构

`Release` workflow 包含两个 job：

1. `changelog`（Ubuntu）  
   读取 `target_commitish` 与上一个 tag 之间的提交，按 `feat` / `fix` / 其他分组，产出 `needs.changelog.outputs.body`。
2. `build`（矩阵构建）  
   并行构建 4 个目标平台：
   - `aarch64-apple-darwin`（macOS-ARM64）
   - `x86_64-apple-darwin`（macOS-x64）
   - `x86_64-unknown-linux-gnu`（Linux-x64）
   - `x86_64-pc-windows-msvc`（Windows-x64）

## 4. `build` job 详细流程（release 与 prerelease 共用）

每个矩阵目标执行以下步骤：

1. `actions/checkout@v4`
2. `actions/setup-node@v4`（Node 20）
3. `dtolnay/rust-toolchain@stable`
4. `swatinem/rust-cache@v2`
5. 从 workflow 输入 `version` 同步版本号到 `package.json` 与 `src-tauri/Cargo.toml`
6. 根据 `is_prerelease` 自动选择 environment：
   - `false`：`release`
   - `true`：`prerelease`
7. 检测签名 secrets 是否齐全，判定 `signed/unsigned` 模式
8. 若 unsigned：自动关闭 updater artifacts 和 macOS signing identity
9. Linux 目标安装系统依赖（仅 `ubuntu-22.04`）
10. signed 模式下才执行 Apple 证书导入与 API key 写入（macOS）
11. macOS signed 模式会从导入证书自动解析 `Developer ID Application` identity
12. macOS signed 模式会先对 `src-tauri/resources/zeroclaw/darwin-{aarch64,x64}/zeroclaw` 显式 `codesign --timestamp --options runtime`
13. `npm ci`
14. 计算构建参数（Windows prerelease 追加 `--bundles nsis`）
15. 执行 Tauri signed build（此阶段只做签名，不做内置 notarize）
16. macOS signed 额外定位 `.app/.dmg`，输出 preflight `codesign` 诊断信息
17. 显式调用 `xcrun notarytool submit`，记录 submission id
18. 轮询 `xcrun notarytool info`（20s 间隔，最大 40 分钟）并实时输出状态；失败时抓取 `notarytool log`
19. notarization Accepted 后执行 `stapler staple/validate`（app + dmg），并 `--clobber` 覆盖上传 notarized DMG
20. 上传 notarization 诊断产物（`notary-*.json` / `notary-*.log`）供排障
21. unsigned 模式将 release 资产重命名为 `*-unsigned.*`
22. 上传 Windows portable（unsigned 模式同样加后缀）
23. macOS 清理临时 keychain 与 API key 文件

## 5. Release 与 Prerelease 的差异

两者流程主体一致，差异点如下：

1. GitHub Release 元数据
   - `prerelease: false`（正式版）
   - `prerelease: true`（预发布）

2. 绑定的 environment
   - `release`（通常对应稳定版本 tag）
   - `prerelease`（通常对应带预发布后缀 tag）

3. Windows 打包参数
   - prerelease 下会额外加 `--bundles nsis`
   - 正式版不加这个额外参数（维持默认 bundles）

4. tag 命名约定
   - 正式版一般为 `vX.Y.Z`
   - 预发布一般为 `vX.Y.Z-alpha.N / beta.N / rc.N`

## 6. 签名决策逻辑（关键）

签名由 secrets 是否齐全决定，而不是仅看 release/prerelease：

关键点：

1. 同时满足以下 secrets 时进入 signed 模式：
   - `TAURI_SIGNING_PRIVATE_KEY`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
   - `APPLE_CERTIFICATE`
   - `APPLE_CERTIFICATE_PASSWORD`
   - `APPLE_API_KEY`
   - `APPLE_API_ISSUER`
   - `APPLE_API_KEY_CONTENT`
2. `APPLE_SIGNING_IDENTITY` 不再是强依赖：workflow 会优先从证书自动解析并注入
3. 只要任一必需项缺失，自动进入 unsigned 模式：
   - 不做 Apple 导入/公证步骤
   - Tauri 配置自动关闭签名相关设置
   - 上传产物名追加 `-unsigned`
4. signed 模式保持原命名（不加后缀）

## 7. 必需 Secrets（发布签名相关）

若希望发布为 signed，至少需要：

1. `TAURI_SIGNING_PRIVATE_KEY`
2. `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
3. `APPLE_CERTIFICATE`
4. `APPLE_CERTIFICATE_PASSWORD`
5. `APPLE_API_KEY`
6. `APPLE_API_ISSUER`
7. `APPLE_API_KEY_CONTENT`

缺少任意一个不会直接失败，而是自动降级为 unsigned 构建并在资产名上标记。

## 8. 与其他 Workflow 的签名行为对比

1. `pr-build.yml`  
   明确是 **unsigned development builds**（用于 PR 测试，不是发布签名产物）。
2. `ci.yml` / `e2e.yml` / `coverage.yml` / `bump-version.yml`  
   无 Apple Developer 签名流程。

## 9. 典型发布操作建议

1. 先确认版本号与 tag 语义
   - 正式版: `vX.Y.Z`
   - 预发布: `vX.Y.Z-beta.N`
2. 手动触发 `Bump Version`，选择 `patch/minor/major/custom`
3. 等待 `Bump Version` 的 `Test CI` 与 `Package CI` 全部通过
4. 确认 `Commit and Trigger Draft Release` 成功（此时尚未创建 git tag）
5. 在 `Release` workflow 中核对 4 平台矩阵构建
6. 在 draft release 中验证产物、签名和说明
7. 点击 Publish（此时 GitHub 创建 `vX.Y.Z` tag 并正式发布）
