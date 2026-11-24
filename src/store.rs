//! Tiny JSON persistence under %LOCALAPPDATA%\Lumen. No serde derive, no database.
use serde_json::Value;
use std::{fs, path::PathBuf};

pub fn dir() -> PathBuf {
    let d = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Lumen");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn load(name: &str) -> Value {
    fs::read_to_string(dir().join(name))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

pub fn save(name: &str, v: &Value) {
    let p = dir().join(name);
    let tmp = p.with_extension("tmp");
    if fs::write(&tmp, v.to_string()).is_ok() {
        let _ = fs::rename(&tmp, &p);
    }
}
