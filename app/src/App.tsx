import { useEffect, useRef } from "react";
import { Notices, StatusBar, TitleBar, ToolStrip } from "./components/Chrome";
import { ApplyChangesDialog } from "./dialogs/ApplyChangesDialog";
import { ApplyRecipeDialog } from "./dialogs/ApplyRecipeDialog";
import { SettingsDialog } from "./dialogs/SettingsDialog";
import { useApp } from "./state";
import { GraphView } from "./views/GraphView";
import { PackagesView } from "./views/PackagesView";
import { ProblemsView } from "./views/ProblemsView";
import { TasksView } from "./views/TasksView";

export function App() {
  const { state, actions } = useApp();

  // settings.index.syncOnStart: fetch the remote sources once at startup.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (state.settings.index.syncOnStart) actions.refresh();
  }, [state.settings.index.syncOnStart, actions]);

  const { dialog } = state;
  return (
    <div className="app">
      {/* Inert while a dialog is open: no Tab, click or screen reader reaches the page behind it. */}
      <div className="app-main" inert={dialog !== null}>
        <TitleBar />
        <div className="app-body">
          <ToolStrip />
          {state.view === "packages" && <PackagesView />}
          {state.view === "graph" && <GraphView />}
          {state.view === "problems" && <ProblemsView />}
          {state.view === "tasks" && <TasksView />}
        </div>
        <StatusBar />
      </div>
      {dialog?.kind === "plan" && <ApplyChangesDialog plan={dialog.plan} options={dialog.options} />}
      {dialog?.kind === "recipes" && <ApplyRecipeDialog />}
      {dialog?.kind === "settings" && <SettingsDialog section={dialog.section} />}
      <Notices />
    </div>
  );
}
