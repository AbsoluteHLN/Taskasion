// 将本机 HLN ui-system 引擎的 dist 构建产物刷新进 vendor/ 快照(引擎是唯一上游)。
// 引擎 dist 路径因机器而异,不写死在仓库里,用 HLN_ENGINE_DIST 环境变量指定。
import { cpSync, existsSync } from "node:fs";
import path from "node:path";

const engineDist = process.env.HLN_ENGINE_DIST;
if (!engineDist || !existsSync(engineDist)) {
  console.error("未设置 HLN_ENGINE_DIST(指向 HLN ui-system 引擎的 dist 目录),跳过刷新。");
  process.exit(1);
}
cpSync(engineDist, path.resolve("vendor/hln-ui-system-v2.3"), { recursive: true });
console.log("vendor/hln-ui-system-v2.3 快照已刷新。");
