// 将本机 HLN ui-system 引擎的 dist 构建产物刷新进 vendor/ 快照(引擎是唯一上游)。
import { cpSync, existsSync } from "node:fs";
import path from "node:path";

const engineDist = "E:/projectHLN/ui-system/v2.3/dist";
if (!existsSync(engineDist)) {
  console.error(`未找到 HLN 引擎 dist: ${engineDist}`);
  process.exit(1);
}
cpSync(engineDist, path.resolve("vendor/hln-ui-system-v2.3"), { recursive: true });
console.log("vendor/hln-ui-system-v2.3 快照已刷新。");
