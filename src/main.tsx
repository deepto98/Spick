import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

const windowKind =
  new URLSearchParams(window.location.search).get("window") === "hud"
    ? "hud"
    : "main";
document.documentElement.dataset.spickWindow = windowKind;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
