//! Report for someone testing an experimental model: a text file with the version, the
//! OS, the model and the tail of hid.log, ready to attach to the "Device report" form.
//! Contains no personal data: the log only holds HID bytes.

use std::path::PathBuf;

use crate::devices::{Model, Status};

/// GitHub form where the report is attached.
pub const FORM_URL: &str = "https://github.com/esdedev/nogenuity/issues/new?template=device-report.yml";

/// Last lines of hid.log that are included (a few minutes of use).
const LOG_LINES: usize = 600;

pub fn text(model: &Model, firmware: u16, status: Option<Status>) -> String {
    let log = crate::sys::data_dir()
        .and_then(|d| std::fs::read_to_string(d.join("hid.log")).ok())
        .unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    let tail = lines[lines.len().saturating_sub(LOG_LINES)..].join("\n");
    format!(
        "Nogenuity device report\n\
         =====================\n\
         App version: {}\n\
         OS: {} ({})\n\
         Model: {}\n\
         USB ID: {:04X}:{:04X}, interface {}\n\
         Firmware: {firmware:04X}\n\
         Tested model: {}\n\
         Capabilities: {:?}\n\
         Last status: {status:?}\n\
         \n\
         hid.log (last {} lines)\n\
         -----------------------\n\
         {tail}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        model.name,
        model.vid,
        model.pid,
        model.interface,
        if model.tested { "yes" } else { "no" },
        model.caps,
        LOG_LINES.min(lines.len()),
    )
}

/// Saves the report in the data folder and returns its path.
pub fn save(model: &Model, firmware: u16, status: Option<Status>) -> Option<PathBuf> {
    let dir = crate::sys::data_dir()?;
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("nogenuity-report.txt");
    std::fs::write(&path, text(model, firmware, status)).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::MODELS;

    #[test]
    fn report_names_the_model_and_its_id() {
        let r = text(MODELS[0], 0x4110, None);
        assert!(r.contains("HyperX Cloud Alpha S"));
        assert!(r.contains("0951:16ED, interface 5"));
        assert!(r.contains("Firmware: 4110"));
    }
}
