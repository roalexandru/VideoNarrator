import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";

// Disable browser context menu except on text inputs (applies to both WebView2 and WebKit)
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (t.tagName !== "INPUT" && t.tagName !== "TEXTAREA" && !t.isContentEditable) {
    e.preventDefault();
  }
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
