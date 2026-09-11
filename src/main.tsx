import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import TaskbarStatus from "./TaskbarStatus";
import "./styles.css";
import { getCurrentWindow } from "@tauri-apps/api/window";

const isTaskbarStatus = getCurrentWindow().label === "taskbar-status";
if (isTaskbarStatus) document.body.classList.add("taskbar-body");
const Root = isTaskbarStatus ? TaskbarStatus : App;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
