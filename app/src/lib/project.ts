import { sameProjectPath } from "./format";

/** What taking in a snapshot means for the state that belongs to a project. */
export interface ProjectSwitch {
  /** The snapshot is for another project than the one before (or for the first one). */
  switched: boolean;
  /**
   * Queued changes, the task focus and a dialog prepared for the old project are dropped. Not so for the first
   * project that is opened, and never for the same project again: opening the open project from the Recent list,
   * or from the Recipe dialog with another spelling of its path, must not lose what the user queued.
   */
  dropsProjectState: boolean;
}

/** `previous` and `next` are the paths of the projects of the snapshots; `null` is "no project". */
export function projectSwitch(previous: string | null, next: string | null): ProjectSwitch {
  const switched = !sameProjectPath(previous, next);
  return { switched, dropsProjectState: switched && previous !== null };
}
