# Shunno Browser for Android (`v2.24.0`)

Lightweight, privacy-first Android browser built with **Kotlin + Jetpack Compose** and a native **Rust Core (`libshunno_core.so`)** featuring Brave's `adblock-rust` filter engine, a 32-thread HTTP Range download manager, YouTube ad stripping, background audio playback, Picture-in-Picture, and an Apple Cupertino bottom toolbar + configurable top/bottom address capsule.

## Direct Download
- **[Download ShunnoBrowser.apk](https://github.com/asanaim/ShunnoBrowser-Android/releases/latest/download/ShunnoBrowser.apk)**

## Architecture
- **`app/` (Kotlin + Jetpack Compose):** Cupertino UI, bottom menu (`•••`), configurable bottom/top search & address bar, card-grid tab switcher with tab groups, WebView request interception, `MediaPlaybackService`, and hardware `DashMediaMuxer`.
- **`rust-core/` (Rust + JNI):** Cross-compiled to `arm64-v8a` (`libshunno_core.so`) with `opt-level = "z"`, LTO, and symbol stripping—reusing `adblock-rust`, `downloader.rs`, `sanitizer.rs`, `https_guard.rs`, `permissions.rs`, `userscripts.rs`, `importer.rs`, and `updater.rs`.

## Automated GitHub Actions Build
Every push to `main` runs `cargo test`, compiles `libshunno_core.so` via `cargo-ndk`, builds `ShunnoBrowser.apk` via Gradle 8.7, and attaches it to **Releases -> Latest**.
