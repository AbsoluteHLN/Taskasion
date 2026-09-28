import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
// HLN ui-system v2.3 战术矢量引擎(tokens + themes + base 一体包),先于业务样式加载;
// 主题定义用 v3 的 9 套 Euclidean 主题(引擎自带的 6 套 v2.3 主题不再使用,但保留在引擎包内)
import "@hln-ui/hln-ui-system-v2.3.css";
import "@hln-ui/hln-v3-themes.css";
import App from "./App";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
