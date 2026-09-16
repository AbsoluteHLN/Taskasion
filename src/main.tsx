import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
// HLN ui-system v2.3 战术矢量引擎(tokens + themes + base 一体包),先于业务样式加载
import "@hln-ui/hln-ui-system-v2.3.css";
import App from "./App";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
