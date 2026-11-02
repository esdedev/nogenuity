//! Nogenuity: open control for HyperX headsets. Windows and Linux.
//! `--tray` starts hidden in the tray (that's how it starts with the system).

#![cfg_attr(windows, windows_subsystem = "windows")]

mod backend;
mod devices;
mod eq;
mod hid;
mod i18n;
mod icon;
#[cfg(test)]
mod winres;
mod layout;
mod report;
mod settings;

// One implementation per OS of: HID access and data, audio (balance) and UI
#[cfg_attr(windows, path = "sys_windows.rs")]
#[cfg_attr(target_os = "linux", path = "sys_linux.rs")]
mod sys;
#[cfg_attr(windows, path = "audio_windows.rs")]
#[cfg_attr(target_os = "linux", path = "audio_linux.rs")]
mod audio;
#[cfg_attr(windows, path = "ui_windows.rs")]
#[cfg_attr(target_os = "linux", path = "ui_linux.rs")]
mod ui;

fn main() {
    // No console on Windows: any panic is saved to crash.log, in the data folder
    std::panic::set_hook(Box::new(|info| {
        if let Some(dir) = sys::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let bt = std::backtrace::Backtrace::force_capture();
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = writeln!(f, "{info}\n\n{bt}\n----");
            }
        }
    }));

    // install.sh: points the session startup entry to the installed copy and exits
    if std::env::args().any(|a| a == "--refresh-autostart") {
        sys::refresh_autostart(true);
        return;
    }

    // install.sh (Linux): installs the udev rule with pkexec, same as the «Grant access» button
    #[cfg(target_os = "linux")]
    if std::env::args().any(|a| a == "--grant-access") {
        let tr = i18n::t();
        let (code, text) = match sys::grant_access() {
            Ok(()) => (0, tr.access_granted),
            Err(sys::GrantError::Cancelled) => (1, tr.access_cancelled),
            Err(sys::GrantError::NoPkexec) => (1, tr.access_no_pkexec),
            Err(sys::GrantError::Failed) => (1, tr.access_failed),
        };
        eprintln!("{text}");
        std::process::exit(code);
    }

    let start_hidden = std::env::args().any(|a| a == "--tray");
    if !ui::claim_single_instance() {
        return; // already running: it has been asked to show its window
    }

    // First launch: start with the system (unless run from the build folder)
    let s = settings::Settings::load();
    let from_build_dir = std::env::current_exe()
        .is_ok_and(|p| p.components().any(|c| c.as_os_str().to_string_lossy().ends_with("target")));
    if !s.existed {
        if !from_build_dir {
            settings::set_autostart(true);
        }
        s.save();
    } else {
        // The startup entry points to an executable that no longer exists (moved, deleted): point it to this one
        sys::refresh_autostart(false);
    }

    ui::run(start_hidden);
}
