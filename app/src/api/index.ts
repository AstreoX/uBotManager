import type { Backend, Platform } from "./backend";
import { createMockBackend } from "./mock";
import { readMockParams, type MockParams } from "./mockParams";
import { tauriBackend, tauriPlatform } from "./tauri";
import type { Change } from "./types";

export type { Backend, Platform, TaskLogEvent, Unlisten } from "./backend";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const browserPlatform: Platform = {
  isTauri: false,
  minimizeWindow: () => undefined,
  toggleMaximizeWindow: () => undefined,
  closeWindow: () => undefined,
  onMaximizedChange: () => () => undefined,
  pickProjectFile: async (title) => window.prompt(title)?.trim() || null,
  openUrl: (url) => void window.open(url, "_blank", "noopener"),
  openPath: () => undefined,
  revealPath: () => undefined
};

/** Mock-only screenshot parameters; empty inside Tauri. */
export const mockParams: MockParams = isTauri ? readMockParams("") : readMockParams(window.location.search);

const mock = isTauri ? null : createMockBackend(mockParams);

export const api: Backend = mock ? mock.backend : tauriBackend;
export const platform: Platform = isTauri ? tauriPlatform : browserPlatform;

/** Pending changes the app starts with (the design scenario queues an install in the mock). */
export const initialChanges: Change[] = mock ? mock.initialChanges : [];
