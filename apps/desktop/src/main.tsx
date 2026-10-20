import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

if (import.meta.env.DEV) {
  const ms = new URLSearchParams(location.hash.slice(1)).get("delay");
  if (ms) {
    const img = new Image();
    img.src = `/__delay?ms=${ms}`;
    document.body.appendChild(img);
  }
}
import { invoke } from "@tauri-apps/api/core";

const report = (m: string) => invoke("log", { message: m }).catch(() => {});
window.addEventListener("error", (e) => report(`error: ${e.message} at ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled: ${String(e.reason)}`));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
