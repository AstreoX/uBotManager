import "@fontsource-variable/inter";
import "@fontsource/jetbrains-mono";
import "./theme.css";
import "./app.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { api } from "./api";
import { setLanguage } from "./i18n";
import { AppProvider, applyTheme } from "./state";

async function start(): Promise<void> {
  const container = document.getElementById("root");
  if (!container) return;
  const root = createRoot(container);
  try {
    const settings = await api.getSettings();
    setLanguage(settings.language);
    applyTheme(settings.theme);
    document.title = "uBot Manager";
    const [snapshot, tasks] = await Promise.all([api.getSnapshot(), api.listTasks()]);
    root.render(
      <StrictMode>
        <AppProvider snapshot={snapshot} settings={settings} tasks={tasks}>
          <App />
        </AppProvider>
      </StrictMode>
    );
  } catch (error) {
    root.render(<pre className="startup-error">{String(error)}</pre>);
  }
}

void start();
