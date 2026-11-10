package com.lados.desktop.autofill

import android.webkit.JavascriptInterface

/** Called from the Tauri webview while a vault is unlocked / locked. */
class AutofillJsBridge {
  @JavascriptInterface
  fun publish(json: String) {
    try {
      AutofillVaultCache.publish(json)
    } catch (_: Exception) {
      AutofillVaultCache.clear()
    }
  }

  @JavascriptInterface
  fun clear() {
    AutofillVaultCache.clear()
  }

  @JavascriptInterface
  fun isReady(): Boolean = AutofillVaultCache.unlocked
}
