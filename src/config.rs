//! Local configuration lives in `~/.sargineer/config.toml` (mode 0600),
//! beside the journal, the pending spool and the cache. The token comes
//! from that file or from `--token` on the command line — never from an
//! agent's own settings and never from the environment.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cli::Cli;
use crate::error::{Result, SargError};

pub const DEFAULT_URL: &str = "https://sargineer.com";

/// The name of the top-level url/token pair: the real sargineer.com.
pub const PROD: &str = "prod";

/// A named server besides prod (`sarg env dev --url … --token …`). Each
/// has its own token and its own cache/spool under `~/.sargineer/envs/`.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Env {
    pub url: Option<String>,
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub url: Option<String>,
    pub token: Option<String>,
    /// os / arch / distro tags, attached as `host` on lessons.
    pub host_tags: Vec<String>,
    /// Terms that must never leave this machine.
    pub deny_terms: Vec<String>,
    /// Local project name → the `project` value sent to sargineer.
    pub project_aliases: BTreeMap<String, String>,
    pub default_project: Option<String>,
    /// Projects registered on this machine: name → directory holding sarg.yaml.
    pub projects: BTreeMap<String, String>,
    /// Server version seen on the last /api fetch, for drift notices.
    pub last_seen_version: Option<String>,
    /// The version before that, so `sarg changelog` defaults to what you missed.
    pub previous_version: Option<String>,
    /// Which server verbs talk to: a name in `envs`, or unset for prod.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// Other servers, e.g. a local dev instance: name → url + token.
    pub envs: BTreeMap<String, Env>,
}

#[derive(Clone, Debug)]
pub struct Paths {
    /// `~/.sargineer`
    pub sargineer_dir: PathBuf,
    pub config_file: PathBuf,
    pub cache_dir: PathBuf,
    /// Notes written while offline, waiting for `sarg sync`.
    pub pending_dir: PathBuf,
    pub home: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
            .ok_or_else(|| SargError::usage("cannot determine the home directory"))?;
        let sargineer_dir = home.join(".sargineer");
        Ok(Paths {
            config_file: sargineer_dir.join("config.toml"),
            cache_dir: sargineer_dir.join("cache"),
            pending_dir: sargineer_dir.join("pending"),
            sargineer_dir,
            home,
        })
    }

    /// Cache and spool for a named env live apart from prod's, so a dev
    /// server's metadata never answers for sargineer.com and a note spooled
    /// for one never lands on the other.
    pub fn for_env(&self, env: &str) -> Paths {
        if env == PROD {
            return self.clone();
        }
        let root = self.sargineer_dir.join("envs").join(env);
        Paths {
            cache_dir: root.join("cache"),
            pending_dir: root.join("pending"),
            ..self.clone()
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Config> {
        match fs::read_to_string(&paths.config_file) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::create_dir_all(&paths.sargineer_dir)?;
        let body = toml::to_string_pretty(self)?;
        fs::write(&paths.config_file, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&paths.config_file, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Set a dotted key from a string. Lists are comma-separated.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let list = |v: &str| -> Vec<String> {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        match key {
            "url" => self.url = Some(value.trim_end_matches('/').to_string()),
            "token" => self.token = Some(value.trim().to_string()),
            "default_project" => self.default_project = Some(value.to_string()),
            "host_tags" => self.host_tags = list(value),
            "deny_terms" => self.deny_terms = list(value),
            k if k.starts_with("project_aliases.") => {
                let name = &k["project_aliases.".len()..];
                if name.is_empty() {
                    return Err(SargError::usage("project_aliases.<name> needs a name"));
                }
                self.project_aliases
                    .insert(name.to_string(), value.to_string());
            }
            k if k.starts_with("projects.") => {
                let name = &k["projects.".len()..];
                if name.is_empty() {
                    return Err(SargError::usage("projects.<name> needs a name"));
                }
                self.projects.insert(name.to_string(), value.to_string());
            }
            "env" => {
                let name = value.trim();
                if name == PROD || name.is_empty() {
                    self.env = None;
                } else if self.envs.contains_key(name) {
                    self.env = Some(name.to_string());
                } else {
                    return Err(SargError::usage(format!(
                        "no env `{name}` — sarg env {name} --url URL [--token TOKEN] to define it"
                    )));
                }
            }
            k if k.starts_with("envs.") => {
                let (name, field) = k["envs.".len()..].split_once('.').unwrap_or(("", ""));
                if name.is_empty() {
                    return Err(SargError::usage("envs.<name>.url or envs.<name>.token"));
                }
                let e = self.envs.entry(name.to_string()).or_default();
                match field {
                    "url" => e.url = Some(value.trim_end_matches('/').to_string()),
                    "token" => e.token = Some(value.trim().to_string()),
                    _ => return Err(SargError::usage("envs.<name>.url or envs.<name>.token")),
                }
            }
            _ => return Err(SargError::usage(format!("unknown config key `{key}`"))),
        }
        Ok(())
    }

    pub fn unset(&mut self, key: &str) -> Result<()> {
        match key {
            "url" => self.url = None,
            "token" => self.token = None,
            "default_project" => self.default_project = None,
            "host_tags" => self.host_tags.clear(),
            "deny_terms" => self.deny_terms.clear(),
            k if k.starts_with("project_aliases.") => {
                self.project_aliases.remove(&k["project_aliases.".len()..]);
            }
            k if k.starts_with("projects.") => {
                self.projects.remove(&k["projects.".len()..]);
            }
            "env" => self.env = None,
            k if k.starts_with("envs.") => {
                let name = &k["envs.".len()..];
                self.envs.remove(name);
                if self.env.as_deref() == Some(name) {
                    self.env = None;
                }
            }
            _ => return Err(SargError::usage(format!("unknown config key `{key}`"))),
        }
        Ok(())
    }

    /// Where the last-seen server version for `env` is kept: prod's on the
    /// config itself, a named env's on its entry.
    pub fn versions_mut(&mut self, env: &str) -> (&mut Option<String>, &mut Option<String>) {
        if env == PROD {
            (&mut self.last_seen_version, &mut self.previous_version)
        } else {
            let e = self.envs.entry(env.to_string()).or_default();
            (&mut e.last_seen_version, &mut e.previous_version)
        }
    }

    pub fn previous_version_for(&self, env: &str) -> Option<String> {
        if env == PROD {
            self.previous_version.clone()
        } else {
            self.envs.get(env).and_then(|e| e.previous_version.clone())
        }
    }

    /// JSON view with the token masked, for `config get` and `--json`.
    pub fn masked_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "token".into(),
                serde_json::Value::String(mask_token(self.token.as_deref())),
            );
            if let Some(envs) = obj.get_mut("envs").and_then(|e| e.as_object_mut()) {
                for (name, e) in envs.iter_mut() {
                    if let Some(o) = e.as_object_mut() {
                        let t = self.envs.get(name).and_then(|x| x.token.as_deref());
                        o.insert("token".into(), serde_json::Value::String(mask_token(t)));
                    }
                }
            }
        }
        v
    }
}

pub fn mask_token(t: Option<&str>) -> String {
    match t {
        None | Some("") => "(unset)".into(),
        Some(t) if t.len() <= 8 => "****".into(),
        Some(t) => format!("****{}", &t[t.len() - 4..]),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TokenSource {
    Flag,
    Config,
    None,
}

impl std::fmt::Display for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TokenSource::Flag => "flag",
            TokenSource::Config => "~/.sargineer/config.toml",
            TokenSource::None => "none",
        })
    }
}

/// What the client actually uses: flag → active env (or prod) → default.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// `prod`, or the name of the env in use.
    pub env: String,
    pub url: String,
    pub token: Option<String>,
    pub token_source: TokenSource,
}

impl Resolved {
    pub fn is_prod(&self) -> bool {
        self.env == PROD
    }
}

pub fn resolve(cli: &Cli, cfg: &Config) -> Result<Resolved> {
    let env = cli
        .env
        .clone()
        .or_else(|| cfg.env.clone())
        .filter(|e| !e.is_empty() && e != PROD);
    let (env, cfg_url, cfg_token) = match env {
        None => (PROD.to_string(), cfg.url.clone(), cfg.token.clone()),
        Some(name) => match cfg.envs.get(&name) {
            Some(e) => {
                let url = e.url.clone().filter(|u| !u.is_empty()).ok_or_else(|| {
                    SargError::usage(format!("env `{name}` has no url — sarg env {name} --url URL"))
                })?;
                (name, Some(url), e.token.clone())
            }
            None => {
                let known: Vec<&str> = cfg.envs.keys().map(String::as_str).collect();
                return Err(SargError::usage(format!(
                    "unknown env `{name}` — sarg env {name} --url URL [--token TOKEN] to define it{}",
                    if known.is_empty() {
                        String::new()
                    } else {
                        format!("; known: prod, {}", known.join(", "))
                    }
                )));
            }
        },
    };
    let url = cli
        .url
        .clone()
        .or(cfg_url)
        .unwrap_or_else(|| DEFAULT_URL.to_string())
        .trim_end_matches('/')
        .to_string();
    let (token, token_source) = match cli.token.clone().filter(|t| !t.is_empty()) {
        Some(t) => (Some(t), TokenSource::Flag),
        None => match cfg_token.filter(|s| !s.is_empty()) {
            Some(t) => (Some(t), TokenSource::Config),
            None => (None, TokenSource::None),
        },
    };
    Ok(Resolved {
        env,
        url,
        token,
        token_source,
    })
}
