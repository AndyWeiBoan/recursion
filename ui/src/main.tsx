import { createRoot } from "react-dom/client";
import App from "./App";
import { connect } from "./client";
import "./styles.css";

connect();
createRoot(document.getElementById("root")!).render(<App />);

// Dev only: the scripted walkthrough for the how-to video (`RECURSION_TOUR=1` on a debug app).
if ((import.meta as { env?: { DEV?: boolean } }).env?.DEV && "__TAURI_INTERNALS__" in window) {
  import("@tauri-apps/api/core")
    .then(({ invoke }) => invoke<string | null>("dev_tour"))
    .then((project) => {
      if (project) import("./devtour").then((t) => t.start(project));
    });
}
