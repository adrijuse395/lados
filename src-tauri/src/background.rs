//! A background thread that wakes up every second to lock the vault after a period of
//! inactivity, clear the clipboard when its time is up and pick up changes that other
//! Lados windows saved to the same file.

use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::clipboard;
use crate::commands::{self, LockReason};
use crate::session::SyncOutcome;
use crate::state::{AppState, Session};
use crate::views::path_text;

const TICK: Duration = Duration::from_secs(1);
const SYNC_INTERVAL: Duration = Duration::from_secs(2);

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut last_sync = Instant::now();
        let mut last_error: Option<String> = None;
        loop {
            thread::sleep(TICK);
            let state = app.state::<AppState>();
            if state.is_quitting() {
                break;
            }
            clipboard::clear_if_due(&app);
            lock_if_idle(&app, &state);
            if last_sync.elapsed() >= SYNC_INTERVAL {
                last_sync = Instant::now();
                sync(&app, &state, &mut last_error);
            }
        }
    });
}

fn lock_if_idle(app: &AppHandle, state: &AppState) {
    let minutes = state.settings().preferences.auto_lock_minutes;
    if minutes > 0 && state.idle_time() >= Duration::from_secs(u64::from(minutes) * 60) {
        commands::lock_now(app, LockReason::Inactivity);
    }
}

/// Each distinct error is reported once, so a file that stays unreadable does not flood the user.
fn sync(app: &AppHandle, state: &AppState, last_error: &mut Option<String>) {
    let (previous_path, result) = {
        let mut session = state.session();
        let Session::Unlocked(open) = &mut *session else {
            return;
        };
        (open.path().to_path_buf(), open.sync())
    };

    match result {
        Ok(SyncOutcome::Unchanged) => {}
        Ok(SyncOutcome::Merged { report, local_changed }) => {
            if local_changed {
                let _ = app.emit("vault-changed", report);
            }
        }
        Ok(SyncOutcome::Diverged(copy)) => {
            commands::remember_copy(state, &previous_path, &copy);
            let _ = app.emit("vault-diverged", path_text(&copy));
        }
        Err(error) => {
            let message = error.to_string();
            if last_error.as_deref() != Some(message.as_str()) {
                let _ = app.emit("vault-sync-error", &message);
                *last_error = Some(message);
            }
            return;
        }
    }
    *last_error = None;
}
