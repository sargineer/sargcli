//! stdout is for the answer, stderr is for everything else.

use serde::Serialize;

use crate::error::SargError;

pub fn json<T: Serialize>(v: &T) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("sarg: cannot serialize output: {e}"),
    }
}

/// `label  value` with labels padded to a column.
pub fn kv(label: &str, value: &str) {
    println!("{label:<12} {value}");
}

pub fn report_error(e: &SargError, as_json: bool) {
    if as_json {
        let v = serde_json::json!({
            "error": e.to_string(),
            "kind": e.kind(),
            "hint": e.hint(),
            "exit": e.exit_code(),
        });
        eprintln!("{}", serde_json::to_string(&v).unwrap_or_default());
        return;
    }
    eprintln!("sarg: {e}");
    if let Some(h) = e.hint() {
        eprintln!("  hint: {h}");
    }
    match e {
        SargError::Auth { .. } => {
            eprintln!("  see: sarg auth status · sarg init · sarg doc start")
        }
        SargError::Offline(_) => eprintln!("  see: sarg sync (once online)"),
        _ => {}
    }
}
