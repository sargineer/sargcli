//! Nothing leaves this machine without passing here. Two classes:
//! `deny` terms from config (client/project names that must never be sent —
//! a hard stop, no override) and heuristic PII (home paths, emails, private
//! IPs, and the bearer token itself — blocked, but `--allow-pii` lifts the
//! heuristic half for the times a path genuinely belongs in a lesson).

use serde_json::Value;

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct Finding {
    pub field: String,
    /// "deny" | "token" | "email" | "path" | "ip"
    pub kind: &'static str,
    pub sample: String,
}

impl Finding {
    /// deny terms and the token can never be sent; PII is overridable.
    pub fn hard(&self) -> bool {
        matches!(self.kind, "deny" | "token")
    }
}

/// Walk every string in the payload and report what should not leave.
pub fn scan(payload: &Value, cfg: &Config, token: Option<&str>) -> Vec<Finding> {
    let mut out = Vec::new();
    let deny: Vec<String> = cfg
        .deny_terms
        .iter()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    walk(payload, "", &deny, token, &mut out);
    out
}

fn walk(v: &Value, path: &str, deny: &[String], token: Option<&str>, out: &mut Vec<Finding>) {
    match v {
        Value::String(s) => scan_str(s, path, deny, token, out),
        Value::Array(a) => {
            for (i, e) in a.iter().enumerate() {
                walk(e, &format!("{path}[{i}]"), deny, token, out);
            }
        }
        Value::Object(o) => {
            for (k, e) in o {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                walk(e, &p, deny, token, out);
            }
        }
        _ => {}
    }
}

fn scan_str(s: &str, field: &str, deny: &[String], token: Option<&str>, out: &mut Vec<Finding>) {
    let low = s.to_lowercase();
    for d in deny {
        if low.contains(d) {
            out.push(Finding {
                field: field.to_string(),
                kind: "deny",
                sample: d.clone(),
            });
        }
    }
    if let Some(t) = token {
        if t.len() >= 12 && s.contains(t) {
            out.push(Finding {
                field: field.to_string(),
                kind: "token",
                sample: "the bearer token".into(),
            });
        }
    }
    if let Some(m) = find_email(s) {
        out.push(Finding {
            field: field.to_string(),
            kind: "email",
            sample: m,
        });
    }
    if let Some(m) = find_home_path(s) {
        out.push(Finding {
            field: field.to_string(),
            kind: "path",
            sample: m,
        });
    }
    if let Some(m) = find_private_ip(s) {
        out.push(Finding {
            field: field.to_string(),
            kind: "ip",
            sample: m,
        });
    }
}

/// Which findings stop a send, and the hint to print. `allow_pii` lifts the
/// heuristic classes; deny terms and the token never lift. Shared by every
/// path that writes a note so the rules cannot drift between them.
pub fn refusal(findings: &[Finding], allow_pii: bool) -> Option<(String, &'static str)> {
    let blocking: Vec<&Finding> = findings.iter().filter(|f| f.hard() || !allow_pii).collect();
    if blocking.is_empty() {
        return None;
    }
    let mut msg = String::from("refusing to send — sensitive content:");
    for f in &blocking {
        msg.push_str(&format!("\n  {} in `{}`: {}", f.kind, f.field, f.sample));
    }
    let hint = if blocking.iter().any(|f| f.hard()) {
        "deny_terms and the token can never be sent; edit the note"
    } else {
        "if a path/email/IP genuinely belongs here, re-run with --allow-pii"
    };
    Some((msg, hint))
}

/// Heuristic, not RFC 5322: every `@` is a candidate, and a candidate is an
/// address when it has a local part and a host that reads as a domain or a
/// dotted-quad. Rejected on purpose: software version tags (`rust@1.98.0`,
/// `hibernate@5.6.15.Final` — numeric first label), and scp / git-remote
/// hosts (`git@github.com:org/repo`), which name a service, not a person.
/// Known limit: non-ASCII local parts or domains are not matched.
fn find_email(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let is_local = |c: u8| c.is_ascii_alphanumeric() || b"._%+-".contains(&c);
    let is_dom = |c: u8| c.is_ascii_alphanumeric() || b".-".contains(&c);
    for (at, _) in s.match_indices('@') {
        let mut lo = at;
        while lo > 0 && is_local(bytes[lo - 1]) {
            lo -= 1;
        }
        let mut hi = at + 1;
        while hi < bytes.len() && is_dom(bytes[hi]) {
            hi += 1;
        }
        // A sentence-ending period or dash belongs to the prose, not the host.
        while hi > at + 1 && matches!(bytes[hi - 1], b'.' | b'-') {
            hi -= 1;
        }
        let local = &s[lo..at];
        let dom = &s[at + 1..hi];
        if local.is_empty() || dom.starts_with('.') || !dom.contains('.') {
            continue;
        }
        if local == "git" || bytes.get(hi) == Some(&b':') {
            continue;
        }
        if is_ipv4(dom) {
            return Some(s[lo..hi].to_string());
        }
        let (first, tld) = (
            dom.split('.').next().unwrap_or(""),
            dom.rsplit('.').next().unwrap_or(""),
        );
        if first.bytes().all(|b| b.is_ascii_digit()) {
            continue; // a version, not a host
        }
        if tld.starts_with("xn--") {
            return Some(s[lo..hi].to_string());
        }
        // A TLD is letters only; anything glued after them is prose.
        let alpha = tld.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
        if alpha >= 2 {
            let end = hi - tld.len() + alpha;
            return Some(s[lo..end].to_string());
        }
    }
    None
}

fn is_ipv4(s: &str) -> bool {
    let mut n = 0;
    for label in s.split('.') {
        n += 1;
        if label.is_empty() || label.len() > 3 || !label.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if label.parse::<u16>().map(|v| v > 255).unwrap_or(true) {
            return false;
        }
    }
    n == 4
}

fn find_home_path(s: &str) -> Option<String> {
    for marker in ["/home/", "/Users/"] {
        if let Some(i) = s.find(marker) {
            let rest = &s[i + marker.len()..];
            let user: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                .collect();
            // "/home/" alone (a generic mention) is fine; a named user directory is not.
            if !user.is_empty() {
                return Some(format!("{marker}{user}…"));
            }
        }
    }
    None
}

fn find_private_ip(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut octets = [0u16; 4];
            let mut n = 0usize;
            let mut cur = 0u16;
            let mut have = false;
            let mut j = i;
            while j < bytes.len() && n < 4 {
                let c = bytes[j];
                if c.is_ascii_digit() {
                    // saturate: "256000 baud" is a number, not an octet, and must not overflow
                    cur = cur.saturating_mul(10).saturating_add((c - b'0') as u16);
                    have = true;
                    j += 1;
                } else if c == b'.' && have {
                    octets[n] = cur;
                    n += 1;
                    cur = 0;
                    have = false;
                    j += 1;
                } else {
                    break;
                }
            }
            if have && n == 3 {
                octets[3] = cur;
                n = 4;
            }
            if n == 4 && octets.iter().all(|o| *o <= 255) {
                let (a, b) = (octets[0], octets[1]);
                // RFC 1918, link-local, and the CGNAT block Tailscale hands out.
                let private = a == 10
                    || (a == 192 && b == 168)
                    || (a == 172 && (16..=31).contains(&b))
                    || (a == 169 && b == 254)
                    || (a == 100 && (64..=127).contains(&b));
                if private {
                    let ip = format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3]);
                    if s.contains(&ip) {
                        return Some(ip);
                    }
                }
                i = j;
                continue;
            }
            i = start + 1;
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(deny: &[&str]) -> Config {
        Config {
            deny_terms: deny.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn catches_each_class() {
        let note = json!({
            "title": "Fix for the acme hat",
            "setup": "on /home/alice/code/x with token",
            "fix": "email me at user@example.com",
            "steps": ["ping 192.168.86.26", "ping 8.8.8.8"],
        });
        let f = scan(&note, &cfg(&["acme"]), Some("supersecrettoken123"));
        let kinds: Vec<_> = f.iter().map(|x| x.kind).collect();
        assert!(kinds.contains(&"deny"), "{f:?}");
        assert!(kinds.contains(&"path"), "{f:?}");
        assert!(kinds.contains(&"email"), "{f:?}");
        assert!(kinds.contains(&"ip"), "{f:?}");
        // public DNS is not flagged
        assert!(f.iter().all(|x| x.sample != "8.8.8.8"));
    }

    #[test]
    fn token_in_payload_is_hard() {
        let note = json!({"body": "curl -H 'Authorization: Bearer supersecrettoken123'"});
        let f = scan(&note, &cfg(&[]), Some("supersecrettoken123"));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "token");
        assert!(f[0].hard());
    }

    #[test]
    fn clean_note_passes() {
        let note = json!({
            "title": "esptool write-flash fails at 0x0 on esp32-s3",
            "fix": "erase-flash first",
            "steps": ["esptool erase-flash", "esptool write-flash 0x0 app.bin"],
        });
        assert!(scan(&note, &cfg(&["acme"]), Some("tok")).is_empty());
    }

    #[test]
    fn version_tags_are_not_emails() {
        // The name@version shape of sw tags must not read as an email.
        let note = json!({
            "sw": ["rust@1.98.0", "libc@0.2", "esptool@5.3.1", "meshcore@1.17.0"],
            "fix": "pin torch@2.10.0",
        });
        assert!(scan(&note, &cfg(&[]), None).is_empty());
        // A real address still trips.
        let real = json!({"fix": "mail me@example.com"});
        let f = scan(&real, &cfg(&[]), None);
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].kind, f[0].sample.as_str()),
            ("email", "me@example.com")
        );
    }

    #[test]
    fn version_tag_does_not_mask_a_later_email() {
        let note = json!({"fix": "pin torch@2.10.0, then ask me@example.com"});
        let f = scan(&note, &cfg(&[]), None);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].sample, "me@example.com");
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_address() {
        for text in [
            "Ask support@vendorx.com.",
            "support@vendorx.com--he owns it",
            "(support@vendorx.com)",
        ] {
            let f = scan(&json!({"body": text}), &cfg(&[]), None);
            assert_eq!(f.len(), 1, "{text}: {f:?}");
            assert_eq!(f[0].sample, "support@vendorx.com", "{text}");
        }
    }

    #[test]
    fn word_final_versions_are_not_emails() {
        let note = json!({
            "sw": ["hibernate@5.6.15.Final", "spring-boot@2.3.0.RELEASE", "bundler@2.5.0.pre", "pkg@1.2.dev"],
        });
        assert!(scan(&note, &cfg(&[]), None).is_empty());
    }

    #[test]
    fn git_remotes_are_not_emails() {
        let note = json!({
            "steps": ["git clone git@github.com:org/repo.git", "ssh -T git@github.com", "scp x deploy@build.local:/srv"],
        });
        assert!(
            scan(&note, &cfg(&[]), None).is_empty(),
            "{:?}",
            scan(&note, &cfg(&[]), None)
        );
    }

    #[test]
    fn user_at_host_ip_is_caught() {
        // Tailscale (CGNAT) and public addresses: the email check catches the
        // user@host form, and the IP check catches a bare 100.x address.
        let f = scan(
            &json!({"steps": ["ssh alice@100.101.102.103"]}),
            &cfg(&[]),
            None,
        );
        assert!(
            f.iter()
                .any(|x| x.kind == "email" && x.sample == "alice@100.101.102.103"),
            "{f:?}"
        );
        let f = scan(&json!({"steps": ["ssh root@203.0.113.7"]}), &cfg(&[]), None);
        assert!(f.iter().any(|x| x.kind == "email"), "{f:?}");
        let f = scan(
            &json!({"steps": ["curl http://100.101.102.103:8088"]}),
            &cfg(&[]),
            None,
        );
        assert!(
            f.iter()
                .any(|x| x.kind == "ip" && x.sample == "100.101.102.103"),
            "{f:?}"
        );
        // 100.x outside the CGNAT block is public and stays quiet.
        assert!(scan(&json!({"steps": ["ping 100.20.1.1"]}), &cfg(&[]), None).is_empty());
    }

    #[test]
    fn punycode_tld_is_an_email() {
        let f = scan(
            &json!({"fix": "contact ivan@xn--e1afmkfd.xn--p1ai"}),
            &cfg(&[]),
            None,
        );
        assert_eq!(f.len(), 1, "{f:?}");
    }

    #[test]
    fn refusal_lifts_only_soft_findings() {
        let note = json!({"setup": "/home/alice/x", "title": "the acme hat"});
        let f = scan(&note, &cfg(&["acme"]), None);
        assert!(refusal(&f, false).is_some());
        let (msg, hint) = refusal(&f, true).expect("deny term still blocks");
        assert!(msg.contains("deny") && !msg.contains("path"), "{msg}");
        assert!(hint.contains("never"));
        let only_soft = scan(&json!({"setup": "/home/alice/x"}), &cfg(&[]), None);
        assert!(refusal(&only_soft, true).is_none());
        assert!(refusal(&only_soft, false)
            .unwrap()
            .1
            .contains("--allow-pii"));
    }
}
