import { createRoot } from "react-dom/client";
import App from "./App";
import { useLang } from "./i18n";
import { installClipboardKeys } from "./clipboard";
import { connect } from "./client";
import "./styles.css";

connect();
installClipboardKeys();
// A new language re-renders everything: text is looked up while rendering.
function Root() {
  return <App key={useLang()} />;
}

createRoot(document.getElementById("root")!).render(<Root />);

// Dev only: the scripted walkthrough for the how-to video (`RECURSION_TOUR=1` on a debug app).
if ((import.meta as { env?: { DEV?: boolean } }).env?.DEV && "__TAURI_INTERNALS__" in window) {
  import("@tauri-apps/api/core")
    .then(({ invoke }) => invoke<string | null>("dev_tour"))
    .then((project) => {
      if (project) import("./devtour").then((t) => t.start(project));
    });
}
