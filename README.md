# Codex Win Widget

Codex Win Widget is a small Windows tray app that shows your remaining Codex limits from the taskbar.

It was built to feel like a native Windows flyout, not like a separate dashboard. Click the tray icon and a compact panel opens above the taskbar with the current remaining percentage, weekly remaining percentage, used percentage, reset time, credits, and token usage. Move the mouse away and it disappears.

## What it does

- Shows the current Codex limit as a tray icon.
- Opens a native-looking flyout from the taskbar.
- Puts the remaining percentage first, because that is the number you usually need.
- Shows weekly remaining, used percentage, reset time, credits, daily tokens, lifetime tokens, and streak when Codex provides those values.
- Refreshes automatically every 60 seconds.
- Uses a 60-second in-memory cache, so opening the panel repeatedly does not keep calling Codex.
- Avoids the visible console flash by resolving Codex in-process and skipping shell wrappers when it can launch the underlying `codex.js` script directly.
- Provides a right-click menu for refresh, opening Codex, copying the current status, starting with Windows, and quitting the app.

## Trademark and affiliation

This project is independent and is not made by, endorsed by, sponsored by, or affiliated with OpenAI.

Codex is used here only to describe compatibility with OpenAI's Codex CLI. OpenAI, ChatGPT, GPT, and related names or marks are the property of OpenAI. OpenAI publishes its current trademark guidance in its [brand guidelines](https://openai.com/brand/).

This app does not include Codex, does not redistribute the Codex CLI, and does not provide access to OpenAI services by itself. You need your own Codex installation and your own valid OpenAI/Codex access.

## Requirements

- Windows 10 or Windows 11.
- Rust with Cargo.
- The Codex CLI installed and available on `PATH`.
- A working Codex login in the same Windows user account that runs the widget.

The app talks to the local Codex CLI through `codex app-server`. It does not call a separate web API directly, and it does not store your account data.

## Build

```powershell
cargo build --release
```

The executable is written to:

```text
target\release\codex-win-widget.exe
```

## Run

```powershell
.\target\release\codex-win-widget.exe
```

Once it starts, the app lives in the Windows notification area. There is no main window.

## Use

Left click the tray icon to show the limits panel.

Right click the tray icon to open the menu:

- `Show limits` opens the flyout.
- `Refresh now` forces a fresh Codex read.
- `Open Codex` opens Codex in the browser.
- `Copy status` copies a short status summary to the clipboard.
- `Start with Windows` turns launch-at-login on or off for the current user.
- `Quit` exits the app.

The flyout also closes when it loses focus, when you press `Esc`, or when the mouse leaves the panel.

## Start with Windows

Use `Start with Windows` in the tray menu if you want the widget to open when you sign in.

The setting is stored for the current Windows user under:

```text
HKCU\Software\Microsoft\Windows\CurrentVersion\Run
```

It does not need administrator permissions. Turn the same menu item off to remove the startup entry.

## Configuration

By default, the widget looks for Codex in this order:

1. `codex.cmd`
2. `codex.exe`
3. `codex`

The lookup is done inside the widget. It does not call `where.exe`, `cmd.exe`, PowerShell, or another helper process just to find Codex.

If `codex.cmd` comes from an npm install, the widget skips the batch file and launches Node directly with the installed Codex script. That avoids the short console window that Windows normally shows when a batch file starts. The resolved command is cached, so later refreshes do not repeat the PATH scan.

You can point the widget at a specific Codex command with:

```powershell
$env:CODEX_WIN_WIDGET_CODEX = "C:\Users\you\AppData\Roaming\npm\codex.cmd"
.\target\release\codex-win-widget.exe
```

The override can point to:

- `codex.cmd`, when it is the npm wrapper next to `node_modules\@openai\codex\bin\codex.js`.
- `codex.exe`, when you have a native executable launcher.
- `codex.js`, when you want the widget to launch the Codex script through `node.exe`.

The widget does not run unknown `.cmd`, `.bat`, or `.ps1` wrappers directly. If the wrapper layout is not recognized, the flyout shows an error instead of opening a flashing shell window.

## How it works

The project is intentionally small:

- `src/app_server.rs` resolves Codex without shell helpers, starts `codex app-server`, sends the account and rate-limit requests over stdin, and reads JSON responses from stdout.
- `src/model.rs` turns Codex account, limit, credit, and usage data into the compact view model used by the tray and flyout.
- `src/native.rs` owns the Win32 tray icon, popup window, drawing code, clipboard integration, menu actions, refresh timer, and mouse-leave behavior.
- `src/main.rs` keeps the binary Windows-only.

The UI is drawn with Win32 and GDI. There is no webview, no background service, and no extra runtime beyond the app itself and the Codex CLI it talks to.

## Refresh behavior

The widget refreshes in three situations:

- When the app starts.
- On the 60-second timer.
- When you choose `Refresh now`.

Opening the flyout also asks for data, but it respects the 60-second cache. If the last refresh is still fresh, the panel opens immediately with the cached snapshot.

The Codex command itself is resolved once per app session and reused after that. This keeps refreshes quiet and avoids repeating PATH work every time the panel opens.

## Development checks

Run these before committing changes:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

There is also an ignored integration test that talks to the local Codex app-server:

```powershell
cargo test fetches_local_app_server_snapshot -- --ignored --nocapture
```

That test requires a working Codex installation and login.

## Notes

This is a local Windows utility for Codex users. It depends on the Codex CLI exposing the local `app-server` interface and on the response shape used by the installed CLI version.

If Codex is not installed, not logged in, or not reachable through the local app-server, the widget stays open. The flyout and tooltip show a short error.

## License

This project is released under the [PolyForm Noncommercial License 1.0.0](LICENSE).

The source is public so people can inspect it, learn from it, run it for personal use, and adapt it for noncommercial work. Commercial use is not allowed unless the copyright holder gives separate written permission.

In plain terms: do not sell this app, bundle it into a paid product, use it as part of a commercial service, or repackage it for commercial distribution without permission.
