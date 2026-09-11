import React from "react";
import ReactDOM from "react-dom/client";
// Inter, as Tailscale's own interface uses, bundled rather than fetched: the
// webview's CSP allows nothing from the network, and neither should a font.
import "@fontsource-variable/inter";
import App from "./App";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
