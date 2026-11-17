//! Subscription billing: starting a checkout, cancelling, and undoing a cancellation.

use super::*;

/// Starts a subscription checkout and returns the URL to open. The account is bound server-side
/// (via the checkout's custom data), so the client can never attach a payment to the wrong one.
#[tauri::command]
pub async fn billing_checkout(app: AppHandle) -> Result<String> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to Lados Sync first"))?;
            crate::sync::billing_checkout(&account.server_url, &account.token)
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Cancels the subscription at the end of the paid period (access continues until then).
#[tauri::command]
pub async fn billing_cancel(app: AppHandle) -> Result<()> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to Lados Sync first"))?;
            crate::sync::billing_cancel(&account.server_url, &account.token)
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}

/// Undoes a scheduled cancellation, so the subscription keeps renewing.
#[tauri::command]
pub async fn billing_resume(app: AppHandle) -> Result<()> {
    #[cfg(feature = "sync")]
    {
        run_blocking(move || {
            let state = app.state::<AppState>();
            let account = state
                .settings()
                .sync_account
                .clone()
                .ok_or_else(|| Error::invalid("Sign in to Lados Sync first"))?;
            crate::sync::billing_resume(&account.server_url, &account.token)
        })
        .await
    }
    #[cfg(not(feature = "sync"))]
    {
        let _ = app;
        Err(Error::invalid("Sync is not available in this build"))
    }
}
