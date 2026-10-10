import { hasKey, msgFromEnglish, tm } from "../i18n";

/**
 * Text for a rejected backend call. The core rejects with English text rendered from a message code
 * ("Another task is running."), which is shown in the UI language; a bare code ("task.busy") is translated
 * too. Anything else (git output, internal errors) is shown as is.
 *
 * A rejection can hold several messages, one per line (a plan with more than one error). Each line is
 * translated on its own: the templates match a whole line, and a line that is not one stays as it is.
 */
export function errorText(error: unknown): string {
  const text = typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
  if (/^[a-z]+\.[A-Za-z.]+$/.test(text) && hasKey(`msg.${text}`)) return tm({ code: text, params: {} });
  return text.split(/\r?\n/).map(lineText).join("\n");
}

function lineText(line: string): string {
  const msg = msgFromEnglish(line);
  return msg ? tm(msg) : line;
}
