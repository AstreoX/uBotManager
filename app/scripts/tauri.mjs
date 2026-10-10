// `npm run tauri <command>`: the Tauri CLI for the shell in ../src-tauri. The CLI only searches the
// current folder and its subfolders for tauri.conf.json, so TAURI_APP_PATH points it at the shell.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

process.env.TAURI_APP_PATH ??= fileURLToPath(new URL("../../src-tauri", import.meta.url));

const cli = createRequire(import.meta.url)("@tauri-apps/cli");
cli.run(process.argv.slice(2), "npm run tauri").catch((error) => {
  cli.logError(error.message);
  process.exit(1);
});
