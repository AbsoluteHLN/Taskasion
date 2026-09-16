import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import fs from "node:fs";

// HLN ui-system v2.3(Industrial-Tactical Vector)独立引擎:本机存在引擎仓库时直连其
// dist(引擎改动即时生效),否则回退到仓库内 vendor/ 快照,保证克隆即可构建。
const engineDist = "E:/projectHLN/ui-system/v2.3/dist";
const hlnDist = fs.existsSync(engineDist)
  ? path.resolve(engineDist)
  : path.resolve("vendor/hln-ui-system-v2.3");

// Tauri 前端:固定端口(壳的 devUrl 依赖 14410),构建目标跟随 WebView2
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: { alias: { "@hln-ui": hlnDist } },
  server: { port: 14410, strictPort: true, fs: { allow: [hlnDist] } },
  build: { target: "es2021", outDir: "dist" },
});
