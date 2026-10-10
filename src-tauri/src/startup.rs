//! Startup: opens the project from `--project` (else `settings.lastProject`, which `Manager::new`
//! picks up), shows a quick snapshot from local files and caches, then runs the full
//! `refresh(sync = settings.index.syncOnStart)` in the background (SPEC §1.1, §12).
//!
//! The frontend asks for a snapshot as soon as it loads and runs its own `refresh(sync = true)`
//! when `syncOnStart` is set. [`Startup`] lets `get_snapshot` wait for the quick scan instead of
//! starting a second scan, and answers the frontend's startup sync with the one already running,
//! so the index is fetched once.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::AppHandle;
use ubot_core::Manager;

use crate::events;

/// How long after the startup sync finished a `refresh(sync = true)` is still taken as the
/// frontend's startup request.
const CLAIM_WINDOW: Duration = Duration::from_secs(30);
/// Callers never wait longer than this for a startup phase.
const MAX_WAIT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Opening the project and the quick scan.
    Scanning,
    /// The full refresh (with the remote fetch when `syncOnStart`).
    Refreshing,
    Done,
}

#[derive(Debug)]
struct State {
    phase: Phase,
    /// The startup refresh fetches the remote sources and has not answered a sync request yet.
    claimable_sync: bool,
    done_at: Option<Instant>,
}

#[derive(Debug)]
pub struct Startup {
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for Startup {
    fn default() -> Self {
        Startup {
            state: Mutex::new(State {
                phase: Phase::Scanning,
                claimable_sync: false,
                done_at: None,
            }),
            changed: Condvar::new(),
        }
    }
}

impl Startup {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn advance(&self, phase: Phase) {
        let mut state = self.lock();
        if phase > state.phase {
            state.phase = phase;
            if phase == Phase::Done {
                state.done_at = Some(Instant::now());
            }
        }
        self.changed.notify_all();
    }

    /// Records whether the startup refresh will sync, i.e. can answer one sync request.
    fn set_syncing(&self, sync: bool) {
        self.lock().claimable_sync = sync;
    }

    /// Blocks while the startup is before `phase`.
    pub fn wait_for(&self, phase: Phase) {
        let state = self.lock();
        let _ = self
            .changed
            .wait_timeout_while(state, MAX_WAIT, |state| state.phase < phase);
    }

    /// True once for the first `refresh(sync = true)` that arrives while the startup sync runs or
    /// shortly after it finished: that request is answered by the startup sync. The caller then
    /// waits for [`Phase::Done`].
    pub fn claim_sync(&self) -> bool {
        let mut state = self.lock();
        let recent = match state.done_at {
            Some(done_at) => done_at.elapsed() < CLAIM_WINDOW,
            None => true,
        };
        let claimed = state.claimable_sync && recent;
        state.claimable_sync = false;
        claimed
    }
}

/// Marks the startup done even when the startup thread panics, so no command waits for it.
struct DoneOnDrop<'a>(&'a Startup);

impl Drop for DoneOnDrop<'_> {
    fn drop(&mut self) {
        self.0.advance(Phase::Done);
    }
}

/// Runs the startup on the calling thread (the setup hook spawns one for it).
pub fn run(app: &AppHandle, manager: &Manager, startup: &Startup, project: Option<String>) {
    let _done = DoneOnDrop(startup);

    if std::env::var_os(ubot_core::paths::HOME_ENV).is_none() {
        // Test and smoke runs (UBOT_MANAGER_HOME set) leave the registry alone.
        if let Err(error) = crate::registry::register_current_exe() {
            events::log_error(&format!(
                "Cannot write HKCU\\{}\\{}: {error}",
                crate::registry::KEY,
                crate::registry::VALUE
            ));
        }
    }

    if let Some(path) = project {
        if let Err(error) = manager.select_project(&path) {
            // The last project (if any) stays open.
            events::show_error(app, &error);
        }
    }
    let name = manager
        .project_path()
        .as_deref()
        .and_then(events::project_name);
    events::set_window_title(app, name.as_deref());

    let sync = manager.get_settings().index.sync_on_start;
    startup.set_syncing(sync);
    match manager.rescan() {
        Ok(snapshot) => events::publish_snapshot(app, &snapshot),
        Err(error) => events::log_error(&error),
    }
    startup.advance(Phase::Refreshing);

    match manager.refresh(sync) {
        Ok(snapshot) => events::publish_snapshot(app, &snapshot),
        Err(error) => events::log_error(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn waits_for_phases() {
        let startup = Arc::new(Startup::default());
        let waiter = {
            let startup = startup.clone();
            thread::spawn(move || {
                startup.wait_for(Phase::Refreshing);
                startup.lock().phase
            })
        };
        thread::sleep(Duration::from_millis(50));
        startup.advance(Phase::Refreshing);
        assert_eq!(waiter.join().unwrap(), Phase::Refreshing);

        // Phases only move forward.
        startup.advance(Phase::Scanning);
        assert_eq!(startup.lock().phase, Phase::Refreshing);
        {
            let _done = DoneOnDrop(&startup);
        }
        startup.wait_for(Phase::Done);
        assert_eq!(startup.lock().phase, Phase::Done);
    }

    #[test]
    fn the_startup_sync_answers_one_request() {
        let startup = Startup::default();
        startup.set_syncing(true);
        assert!(startup.claim_sync());
        assert!(!startup.claim_sync(), "only the first request is answered");

        let startup = Startup::default();
        startup.set_syncing(false);
        assert!(
            !startup.claim_sync(),
            "a startup without sync answers nothing"
        );

        let startup = Startup::default();
        startup.set_syncing(true);
        startup.advance(Phase::Done);
        startup.lock().done_at = Some(Instant::now() - CLAIM_WINDOW - Duration::from_secs(1));
        assert!(!startup.claim_sync(), "a late request syncs again");
    }
}
