import React from "react";
import ReactDOM from "react-dom/client";
import { RecorderPill } from "./features/project-setup/RecorderPill";

ReactDOM.createRoot(document.getElementById("recorder-root")!).render(
  <React.StrictMode>
    <RecorderPill />
  </React.StrictMode>,
);
