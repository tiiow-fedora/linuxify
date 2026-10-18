//! Where Linuxify may write. Safety rule #1: stages 1–4 only create files
//! under %LOCALAPPDATA%\Linuxify (plus the user-chosen migration target).

use std::path::PathBuf;

pub fn home() -> Result<PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA").map_err(|_| "LOCALAPPDATA is not set".to_string())?;
    Ok(PathBuf::from(base).join("Linuxify"))
}

pub fn reports_dir() -> Result<PathBuf, String> {
    Ok(home()?.join("reports"))
}
