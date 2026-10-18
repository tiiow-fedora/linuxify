//! `linuxify scan`: gathers everything, read-only.

pub mod hardware;
pub mod programs;
pub mod userdata;

use crate::mapping::{self, AppMap, MappedProgram};
use crate::readiness::{self, Finding};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub generated_at: String,
    pub linuxify_version: String,
    pub computer_name: Option<String>,
    pub app_map_source: String,
    pub winget_used: bool,
    pub hardware: hardware::HardwareFacts,
    pub hardware_error: Option<String>,
    pub programs: Vec<MappedProgram>,
    pub user_data: userdata::UserData,
    pub findings: Vec<Finding>,
}

pub struct ScanOptions {
    pub use_winget: bool,
}

/// Runs every read-only scanner. `progress` gets one line per phase.
pub fn run(map: &AppMap, map_source: String, opts: &ScanOptions, mut progress: impl FnMut(&str)) -> ScanReport {
    progress("Reading installed programs from the registry...");
    let registry = programs::read_registry();
    let winget_rows = if opts.use_winget {
        progress("Asking winget for its list (this can take a few seconds)...");
        programs::run_winget().map(|t| programs::parse_winget_table(&t))
    } else {
        None
    };
    let winget_used = winget_rows.as_ref().is_some_and(|r| !r.is_empty());
    let progs = programs::merge_programs(&registry, winget_rows.as_deref().unwrap_or(&[]));
    let mapped = mapping::classify_all(&progs, map);

    progress("Checking hardware, disks and boot settings...");
    let (hw, hw_error) = hardware::scan();

    progress("Measuring your folders, browser profiles and game libraries...");
    let data = userdata::scan();

    let findings = readiness::assess(&hw, hw_error.as_deref(), &mapped, &data);
    ScanReport {
        generated_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        linuxify_version: env!("CARGO_PKG_VERSION").into(),
        computer_name: std::env::var("COMPUTERNAME").ok(),
        app_map_source: map_source,
        winget_used,
        hardware: hw,
        hardware_error: hw_error,
        programs: mapped,
        user_data: data,
        findings,
    }
}
