import React from "react";
import ReactDOM from "react-dom/client";
import { RecorderOverlay } from "./features/recorder/RecorderOverlay";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <RecorderOverlay />
  </React.StrictMode>
);
