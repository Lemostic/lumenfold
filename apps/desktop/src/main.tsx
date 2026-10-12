import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "./styles/app.css";
import { AppShell } from "./layout/AppShell";
import { bindEditHandler } from "./components/Lightbox";
import { bridge, hostAvailable } from "./lib/ipc";

bindEditHandler(
  hostAvailable
    ? (request) =>
        bridge.editImage(
          request.assetId,
          request.transform,
          request.mode,
          request.enhance ?? false,
        )
    : null,
);

const container = document.getElementById("root");
if (!container) throw new Error("缺少 #root 挂载点");

createRoot(container).render(
  <StrictMode>
    <AppShell />
  </StrictMode>,
);
