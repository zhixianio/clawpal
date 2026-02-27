import { createRoot } from "react-dom/client";
import "./i18n";
import { App } from "./App";

import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <App />,
);

const splash = document.getElementById("boot-splash");
if (splash) {
  requestAnimationFrame(() => {
    splash.classList.add("hidden");
    setTimeout(() => {
      splash.remove();
    }, 260);
  });
}
