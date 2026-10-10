/**
 * Where Tab moves inside a modal dialog with `count` focusable controls when control `index` has focus
 * (-1: the dialog itself). Returns the index to focus, or `null` when the browser's own move stays inside.
 */
export function tabTarget(count: number, index: number, backwards: boolean): number | null {
  if (count === 0) return null;
  if (index < 0) return backwards ? count - 1 : 0;
  if (backwards && index === 0) return count - 1;
  if (!backwards && index === count - 1) return 0;
  return null;
}
