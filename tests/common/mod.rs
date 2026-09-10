//! A throwaway HOME (so `~/.sargineer` is private to the test) and a mock
//! sargineer, per test. No environment leaks in: the token and URL travel
//! as flags, the way a script would pass them.

#![allow(dead_code)]

use std::process::{Command, Output};

use httpmock::prelude::*;
use tempfile::TempDir;

pub struct Sandbox {
    pub _dir: TempDir,
    pub home: std::path::PathBuf,
    pub server: MockServer,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        Sandbox {
            _dir: dir,
            home,
            server: MockServer::start(),
        }
    }

    /// Run with `--url <mock>` unless the args already carry `--url`.
    pub fn sarg(&self, args: &[&str]) -> Output {
        self.sarg_env(args, &[])
    }

    pub fn sarg_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        self.sarg_env_in(args, env, &self.home)
    }

    /// Signed in, run from `dir` (a project directory under the sandbox home).
    pub fn sarg_in_dir(&self, args: &[&str], dir: &std::path::Path) -> Output {
        let mut full = vec!["--token", "tok-123"];
        full.extend_from_slice(args);
        self.sarg_env_in(&full, &[], dir)
    }

    pub fn sarg_env_in(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        dir: &std::path::Path,
    ) -> Output {
        self.run(args, env, dir, true)
    }

    /// No `--url` injected: the config file (and `sarg env`) decide the server.
    pub fn sarg_bare(&self, args: &[&str]) -> Output {
        self.run(args, &[], &self.home, false)
    }

    fn run(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        dir: &std::path::Path,
        inject_url: bool,
    ) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sarg"));
        if inject_url && !args.contains(&"--url") {
            cmd.arg("--url").arg(self.server.base_url());
        }
        cmd.args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            // journal stays on; each test's HOME is a private temp dir, so the
            // guard can read back its own state without polluting anything.
            .env("COLUMNS", "100")
            .current_dir(dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("run sarg")
    }

    /// Signed in as a member: token passed as a flag on this call.
    pub fn sarg_in(&self, args: &[&str]) -> Output {
        let mut full = vec!["--token", "tok-123"];
        full.extend_from_slice(args);
        self.sarg_env(&full, &[])
    }

    pub fn config_path(&self) -> std::path::PathBuf {
        self.home.join(".sargineer/config.toml")
    }

    /// Mock a GET returning a JSON fixture.
    pub fn json_get(&self, path: &str, body: &'static str) -> httpmock::Mock<'_> {
        self.server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        })
    }
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
pub fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}
