//! Building and validating a note against the server's own field rules.
//! The rules come from the cached `/api note_fields`, never a schema baked
//! into the binary — where the two disagree, the server is right.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::error::{Result, SargError};

/// Fields whose value must be a single line (the search titles are built
/// from them). From the field docs, not guessable from the example alone.
const SINGLE_LINE: [&str; 4] = ["title", "symptom", "cause", "fix"];

pub struct Spec {
    pub kind: String,
    pub required: Vec<String>,
    pub statuses: Vec<String>,
    /// Fields the server expects as arrays (steps, hw, sw, host, unverified…).
    pub list_fields: BTreeSet<String>,
    /// The example's title, so we can reject the untouched template.
    pub example_title: Option<String>,
}

/// Read the rules for one kind out of `/api`'s `note_fields`.
pub fn spec_for_kind(note_fields: &Value, kind: &str) -> Spec {
    let arr = |v: Option<&Value>| -> Vec<String> {
        v.and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    // Required: kind-specific list if present, else the top-level required.
    let required = {
        let per_kind = note_fields
            .get("kinds")
            .and_then(|k| k.get(kind))
            .and_then(|k| k.get("required"));
        let r = arr(per_kind);
        if r.is_empty() {
            arr(note_fields.get("required"))
        } else {
            r
        }
    };
    let statuses = arr(note_fields.get("status"));

    // List fields: any example value that is an array, plus the known set.
    let mut list_fields: BTreeSet<String> =
        ["steps", "hw", "sw", "host", "unverified", "refs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    if let Some(ex) = note_fields.get("example").and_then(|e| e.as_object()) {
        for (k, v) in ex {
            if v.is_array() {
                list_fields.insert(k.clone());
            }
        }
    }
    let example_title = note_fields
        .get("example")
        .and_then(|e| e.get("title"))
        .and_then(|t| t.as_str())
        .map(String::from);

    Spec {
        kind: kind.to_string(),
        required,
        statuses,
        list_fields,
        example_title,
    }
}

/// A placeholder like `<exact error>` — the untouched template.
fn has_placeholder(s: &str) -> bool {
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'<' {
            if let Some(close) = s[i + 1..].find('>') {
                let inner = &s[i + 1..i + 1 + close];
                // <...> with words/spaces inside, not an HTML-ish tag with '/' or '='
                if !inner.is_empty()
                    && inner.len() <= 40
                    && !inner.contains('/')
                    && inner.chars().next().unwrap().is_ascii_alphabetic()
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Validate a note. Errors block the send; the returned strings are soft
/// warnings worth showing but not fatal.
pub fn validate(note: &Value, spec: &Spec) -> Result<Vec<String>> {
    let obj = note
        .as_object()
        .ok_or_else(|| SargError::Validation {
            message: "a note must be a JSON object".into(),
            hint: None,
        })?;
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let is_empty = |v: &Value| match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    };

    for req in &spec.required {
        match obj.get(req) {
            None => errors.push(format!("missing required field `{req}`")),
            Some(v) if is_empty(v) => errors.push(format!("required field `{req}` is empty")),
            _ => {}
        }
    }

    if let Some(st) = obj.get("status").and_then(|s| s.as_str()) {
        if !spec.statuses.is_empty() && !spec.statuses.iter().any(|s| s == st) {
            errors.push(format!(
                "status `{st}` is not one of {}",
                spec.statuses.join(", ")
            ));
        }
    }

    for (k, v) in obj {
        if spec.list_fields.contains(k) && !v.is_array() && !v.is_null() {
            errors.push(format!("`{k}` must be a list, not a single value"));
        }
        if SINGLE_LINE.contains(&k.as_str()) {
            if let Some(s) = v.as_str() {
                if s.contains('\n') {
                    errors.push(format!("`{k}` must be a single line — long detail goes in body"));
                }
            }
        }
        // Placeholders anywhere are the untouched template.
        let mut check_ph = |s: &str| {
            if has_placeholder(s) {
                errors.push(format!("`{k}` still has a <placeholder> — fill it in"));
            }
        };
        match v {
            Value::String(s) => check_ph(s),
            Value::Array(a) => a.iter().filter_map(|e| e.as_str()).for_each(&mut check_ph),
            _ => {}
        }
    }

    if let Some(t) = obj.get("title").and_then(|t| t.as_str()) {
        if t.trim().chars().count() < 12 {
            errors.push("title is too short to be found later — write the search someone will type".into());
        }
        if spec.example_title.as_deref() == Some(t) {
            errors.push("title is still the example — write your own".into());
        }
        // Soft: a lesson title should carry the verbatim error.
        if spec.kind == "lesson" {
            let has_signal = t.chars().any(|c| "0123456789".contains(c))
                || t.contains("Error")
                || t.contains("error")
                || t.contains("fail")
                || t.contains("0x");
            if !has_signal {
                warnings.push("title has no error text or version — will it match the search someone types?".into());
            }
        }
    }

    if let Some(b) = obj.get("body").and_then(|b| b.as_str()) {
        let words = b.split_whitespace().count();
        if words > 80 {
            warnings.push(format!("body is {words} words; keep it under ~80 — steps carry the detail"));
        }
    }

    if !errors.is_empty() {
        return Err(SargError::Validation {
            message: errors.join("; "),
            hint: Some("sarg api --fields shows the rules; sarg doc share explains them".into()),
        });
    }
    Ok(warnings)
}

// ---- TOML round-trip for `--edit`, so a human edits a friendly template ----

pub fn json_to_toml(v: &Value) -> toml::Value {
    match v {
        Value::Null => toml::Value::String(String::new()),
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else {
                toml::Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(a.iter().map(json_to_toml).collect()),
        Value::Object(o) => {
            let mut t = toml::map::Map::new();
            for (k, val) in o {
                if !val.is_null() {
                    t.insert(k.clone(), json_to_toml(val));
                }
            }
            toml::Value::Table(t)
        }
    }
}

pub fn toml_to_json(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            let mut o = Map::new();
            for (k, val) in t {
                let jv = toml_to_json(val);
                // Drop empty strings/arrays so an untouched optional field is simply absent.
                let empty = matches!(&jv, Value::String(s) if s.trim().is_empty())
                    || matches!(&jv, Value::Array(a) if a.is_empty());
                if !empty {
                    o.insert(k.clone(), jv);
                }
            }
            Value::Object(o)
        }
    }
}

/// Parse a note file: JSON if it opens with `{`, otherwise TOML.
pub fn parse_note(text: &str) -> Result<Value> {
    let t = text.trim_start();
    if t.starts_with('{') {
        Ok(serde_json::from_str(text)?)
    } else {
        let tv: toml::Value = toml::from_str(text)?;
        Ok(toml_to_json(&tv))
    }
}

/// A TOML template for `--edit`, seeded from `seed` (the /api example for a
/// new note, or the current note for an edit).
pub fn edit_template(seed: &Value, kind: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("# sarg {kind} — fill this in, save, and close the editor.\n"));
    out.push_str("# Lines you leave blank or as a <placeholder> are dropped.\n");
    out.push_str("# Lists use [ ]; body can span lines with triple quotes.\n\n");
    let tv = json_to_toml(seed);
    let body = toml::to_string_pretty(&tv).unwrap_or_default();
    out.push_str(&body);
    out
}
