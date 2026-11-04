//! The JSON that the PowerShell hardware probe prints, turned into facts and findings.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::scan::hardware::{self, Probe};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let Ok(probe) = serde_json::from_str::<Probe>(text.trim().trim_start_matches('\u{feff}')) else { return };
    let _ = hardware::bitlocker_volumes(&probe);
    let _ = hardware::detect_rst_vmd(&probe.storage_controllers, &probe.disks);
    let _ = hardware::wifi_from_raw(&probe.net_adapters);
    let _ = hardware::interpret(probe, hardware::RegistryBootFacts::default());
});
