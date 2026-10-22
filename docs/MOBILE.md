# Lados on mobile (Android first)

Same repository as desktop. One product, one `.lados` format, optional merge later — **no cloud**.

You keep coding in Cursor. **Android Studio** is only the Android SDK toolchain Tauri needs to produce an APK.

## Status

- Strategy: Tauri 2 Mobile in this repo (not a separate `lados-mobile` app).
- Desktop stays the source of truth for vault/crypto/merge.
- Phone UI starts as a responsive adaptation of the existing frontend; a dedicated navigation model can come later.
- `lib.rs` must keep `#[cfg_attr(mobile, tauri::mobile_entry_point)]` on `run()` or Android crashes at launch with `UnsatisfiedLinkError` / missing `Rust.onCreate`.
- Release profile uses `strip = "debuginfo"` (not `"symbols"`) so JNI exports stay in the `.so`.
- Vaults on Android live in the shared folder **`Documents/Lados`** so they **survive uninstall**. App-private storage is wiped by Android on uninstall and must not hold the only copy. **Create and open both use this folder by default** (one standard location).
- The first launch asks for “All files access” (needed to write that public folder on modern Android).
- “Open a vault” lists files already in `Documents/Lados`. “Browse elsewhere…” uses the system picker (often starts in Downloads) and **copies** the chosen `.lados` into `Documents/Lados`. Android’s picker cannot reliably open in a given folder; the in-app list is the standardized path.
- App icon must be the **shield + dragon** mark (`brand/lados-app-icon-1024.png`), not the dragon-only simplification. For Android adaptive icons / installer masks, regenerate from `brand/lados-app-icon-safe-1024.png` (logo inside the ~56% safe zone) via `npx tauri icon brand/lados-app-icon-safe-1024.png`.

## One-time machine setup (Windows)

1. Install [Android Studio](https://developer.android.com/studio) (or `winget install Google.AndroidStudio`).
2. Open Android Studio → **More Actions → SDK Manager** and install:
   - Android SDK Platform (API 34+ is fine)
   - Android SDK Platform-Tools
   - Android SDK Build-Tools
   - NDK (Side by side)
   - Android SDK Command-line Tools
3. Set user environment variables (PowerShell as your user):

```powershell
[System.Environment]::SetEnvironmentVariable("JAVA_HOME", "C:\Program Files\Android\Android Studio\jbr", "User")
[System.Environment]::SetEnvironmentVariable("ANDROID_HOME", "$env:LOCALAPPDATA\Android\Sdk", "User")
$ndk = Get-ChildItem "$env:LOCALAPPDATA\Android\Sdk\ndk" -Directory | Sort-Object Name | Select-Object -Last 1
[System.Environment]::SetEnvironmentVariable("NDK_HOME", $ndk.FullName, "User")
```

Restart the terminal (and Cursor) so the variables load.

4. **Windows Developer Mode** (required): Tauri links the native `.so` with a symlink.  
   Settings → System → For developers → **Developer Mode** → On.  
   Without this, `android:build` fails after Rust finishes compiling.

6. Rust Android targets (once):

```powershell
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
```

## Project commands

From the repo root:

```powershell
# First time only — generates src-tauri/gen/android
npm run android:init

# Dev on emulator or USB device (debugging)
npm run android:dev

# Release APK / AAB
npm run android:build
```

## Install a beta on your phone (no Play Store)

1. On the phone: **Settings → Developer options → USB debugging** (tap Build number 7× on About phone if needed).
2. Plug in USB; accept the debugging prompt.
3. Check the device:

```powershell
& "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" devices
```

4. Build (from repo root, with env vars set):

```powershell
npm run android:build
```

If Windows blocks the symlink step (“Creation symbolic link is not allowed”), turn on **Developer Mode** and rebuild. Until then you can still ship an arm64 APK by copying the native lib and assembling with Gradle (see troubleshooting below).

5. Install:

```powershell
& "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" install -r path\to\app-arm64-release.apk
```

Typical paths:

- Debug (sideload-friendly, auto-signed):  
  `src-tauri/gen/android/app/build/outputs/apk/arm64/debug/app-arm64-debug.apk`
- Release: needs signing; unsigned APK alone will not install.

Sideloading an APK costs **€0**. Google Play ($25 one-time) and Apple Developer (~$99/year) wait until you want store listing / iOS.

### Troubleshooting (Windows)

| Symptom | Fix |
| --- | --- |
| Symlink error after Rust finishes | Enable Developer Mode |
| `Your project path contains non-ASCII characters` | Already mitigated via `android.overridePathCheck=true` in `gen/android/gradle.properties` |
| Gradle `npm.bat` / rust rebuild fails | Ensure Node is on `PATH`, or assemble with `-x rustBuildArm64Release` after copying `liblados_lib.so` into `jniLibs/arm64-v8a` |

## Layout smoke test (without the phone)

After `npm run build`, serve `dist/` and open the phone preview:

```powershell
npx serve dist -p 4173
# Chrome → http://127.0.0.1:4173/mobile-layout-smoke.html
# DevTools → Ctrl+Shift+M → width ≤ 720
```

Check: top bar does not sit under a fake status bar (Android status bar is handled by `MainActivity` fitting system windows), the **Folders** control opens the drawer, and modals stay inside the width.

AVD `lados_phone` exists for full APK installs when the emulator GPU works; on this machine it often hangs — prefer Chrome device mode for CSS and the physical phone for the real shell.

## Product notes

- Mobile and desktop vaults may diverge on purpose.
- To reconcile: merge the two `.lados` files on desktop, then copy the result back to the phone.
- System tray and in-app updater stay **desktop-only**.
- On phone, folders live behind the **Folders** button (drawer); the three-column desktop sidebar is not shown below 720px.
