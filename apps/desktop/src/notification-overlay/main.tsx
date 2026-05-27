import React from "react";
import ReactDOM from "react-dom/client";

import { NotificationOverlayApp } from "@/notification-overlay/notification-overlay-app";
import "@/styles/notification-overlay.css";

document.body.dataset.notificationOverlayRoot = "true";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <NotificationOverlayApp />
  </React.StrictMode>,
);
