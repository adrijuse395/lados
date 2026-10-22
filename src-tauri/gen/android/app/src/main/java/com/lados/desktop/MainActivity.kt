package com.lados.desktop

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.view.View
import android.webkit.WebView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  private var promptedStorageThisSession = false

  override fun onCreate(savedInstanceState: Bundle?) {
    // Keep the WebView below the status/navigation bars. Edge-to-edge draws under
    // system chrome, but Android WebView does not expose CSS env(safe-area-inset-*),
    // so the app top bar would collide with the clock/battery row.
    WindowCompat.setDecorFitsSystemWindows(window, true)
    // Match Lados dark chrome (avoid Material purple defaults).
    val chrome = Color.parseColor("#111418")
    window.statusBarColor = chrome
    window.navigationBarColor = chrome
    WindowInsetsControllerCompat(window, window.decorView).apply {
      isAppearanceLightStatusBars = false
      isAppearanceLightNavigationBars = false
    }
    super.onCreate(savedInstanceState)
    ensureVaultStorageAccess()
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.overScrollMode = View.OVER_SCROLL_NEVER
    webView.isVerticalScrollBarEnabled = false
    webView.isHorizontalScrollBarEnabled = false
    webView.setBackgroundColor(Color.parseColor("#111418"))
  }

  /**
   * Vaults live in public Documents/Lados so they survive uninstall. That requires
   * broad storage access on modern Android (sideload / offline password manager).
   */
  private fun ensureVaultStorageAccess() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      if (Environment.isExternalStorageManager() || promptedStorageThisSession) return
      promptedStorageThisSession = true
      try {
        val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
        intent.data = Uri.parse("package:$packageName")
        startActivity(intent)
      } catch (_: Exception) {
        try {
          startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
        } catch (_: Exception) {
          // Ignore — Rust side will surface a clear error if Documents/Lados is unwritable.
        }
      }
      return
    }

    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
      val needed = arrayOf(
        Manifest.permission.READ_EXTERNAL_STORAGE,
        Manifest.permission.WRITE_EXTERNAL_STORAGE,
      ).filter {
        ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
      }
      if (needed.isNotEmpty()) {
        ActivityCompat.requestPermissions(this, needed.toTypedArray(), 3911)
      }
    }
  }
}
