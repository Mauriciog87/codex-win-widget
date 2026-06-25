#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    codex_win_widget::native::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("codex-win-widget only supports Windows.");
}
