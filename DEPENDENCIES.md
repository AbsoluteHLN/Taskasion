# 开发依赖与端口

## 依赖

- **前端**:Node ≥ 18 + pnpm(依赖见 `package.json`,锁定版本见 `pnpm-lock.yaml`)。
- **桌面壳 + 核心**:Rust + Tauri 2(依赖锁定见 `src-tauri/Cargo.lock`)。核心已用 Rust 重写并并入壳进程,不再需要 Python。

普通开发直接 `pnpm install` 与 `cargo build` 即可(默认走 crates.io / npm registry)。

## 端口规划

- Vite dev server:**14410**(Tauri 壳的 `devUrl` 依赖此端口,固定不变)。
- Core REST:**14411**(全项目统一口径:`src/api.ts` 兜底、壳 CSP、Core 默认一致)。

## 作者本机备注

作者机器使用本地依赖缓存加速安装(全局规则见机器级文档,不入库);不影响上述标准流程。
