# Auto-Update Roadmap (future Tauri GUI)

Status: **research only** — no updater code in this repo yet. This note tells
future-us what to prepare so auto-update works on day one of the desktop app.

Reference: `tauri-plugin-updater` v2 (`tauri-apps/plugins-workspace`).

## How it works (Tauri v2)

1. App registers the plugin at startup (`src-tauri/src/lib.rs`):

   ```rust
   app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
   ```

   with `tauri-plugin-updater = "2"` under desktop-only dependencies.

2. `tauri.conf.json` needs `plugins.updater` = `{ endpoints, pubkey }`
   (both required). The endpoint serves a versioned `latest.json`.

3. Releases are signed: `tauri signer generate` produces a keypair; the
   **private** key lives in CI secrets (`TAURI_SIGNING_PRIVATE_KEY`), the
   **public** key goes into config. `bundle.createUpdaterArtifacts = true`
   makes the bundler emit per-platform update artifacts (installer/zip +
   `.sig` signature files).

4. Frontend checks via `UpdaterExt::check()` (`check()` → `Some(update)` →
   `download_and_install`), typically on startup + a manual menu item.

## What this repo should prepare now

- **Release workflow** (`.github/workflows/release.yml`, later): build the
  Tauri app per OS with `tauri-apps/tauri-action`, which uploads artifacts
  AND generates/updates `latest.json` on the GitHub Release automatically.
- **Artifact naming**: keep default `tauri-action` names
  (`<app>_<version>_<target>.<ext>`, `latest.json`) — the updater expects
  exactly these; do not rename in a custom script.
- **Version source of truth**: GUI app version in `src-tauri/tauri.conf.json`
  (and `src-tauri/Cargo.toml`); keep `kzktdk` sidecar CLI version in
  `Cargo.toml` in lockstep at release time (today both `0.1.0` pre-release).
- **Signing**: generate the keypair when the GUI repo is created; never
  commit the private key; document rotation in this file when it happens.
- **Channels**: one `latest.json` per channel if we ever ship beta
  (`endpoints` supports multiple URLs — stable first, beta later).

## Non-goals

- The `kzktdk` CLI itself gets no self-updater (package managers own that:
  cargo/msi/deb/homebrew).
- No update UI copy in this repo — that lives in the future GUI project.

See also: `docs/GPU_ROADMAP.md`, `DOCUMENTATION.md` §7 (GUI Contract).
