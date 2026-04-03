import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { RecorderWindow } from "./features/project-setup/RecorderWindow";
import "./styles/globals.css";

// Check if this is the recorder window
const isRecorder = window.location.search.includes("view=recorder");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isRecorder ? <RecorderWindow /> : <App />}
  </React.StrictMode>,
);
