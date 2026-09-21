//! A throwaway HOME (so `~/.sargineer` is private to the test) and a mock
//! sargineer, per test. No environment leaks in: the token and URL travel
//! as flags, the way a script would pass them.

#![allow(dead_code)]

use std::process::{Command, Output};

use tempfile::TempDir;

#[allow(unused_imports)] // each test binary only names the subset it calls
pub use fixture::{FixtureServer, Mock, Then, When, GET, POST, PUT};

pub struct Sandbox {
    pub _dir: TempDir,
    pub home: std::path::PathBuf,
    pub server: FixtureServer,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        Sandbox {
            _dir: dir,
            home,
            server: FixtureServer::start(),
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
            // `directories` uses this on Windows if HOME is unavailable.
            .env("USERPROFILE", &self.home)
            // journal stays on; each test's HOME is a private temp dir, so the
            // guard can read back its own state without polluting anything.
            .env("COLUMNS", "100")
            .current_dir(dir);
        keep_os_env(&mut cmd);
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

    /// `sarg ask … | head -1`, the way an agent keeps output short: the
    /// reader goes away mid-write. Returns `head`'s output, not sarg's —
    /// what matters is what sarg managed to record before it stopped.
    #[cfg(unix)]
    pub fn sarg_piped_into_head(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut line = format!(
            "{:?} --url {:?}",
            env!("CARGO_BIN_EXE_sarg"),
            self.server.base_url()
        );
        for a in args {
            line.push_str(&format!(" {a:?}"));
        }
        // A reader that closes stdin at once, so the first write really does
        // get EPIPE. `| head -1` only does that if the output outruns the
        // pipe buffer, which made the test a coin flip.
        line.push_str(" | sh -c 'exec <&-; sleep 0.3'");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(line)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("COLUMNS", "100")
            .current_dir(&self.home);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("run sarg through a pipe")
    }

    pub fn config_path(&self) -> std::path::PathBuf {
        self.home.join(".sargineer/config.toml")
    }

    /// Mock a GET returning a JSON fixture.
    pub fn json_get(&self, path: &str, body: &'static str) -> Mock {
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

/// `env_clear()` on Windows also drops what the OS itself needs in a child:
/// without `SystemRoot` Winsock cannot open a socket (every request fails
/// with "Connect error"), and without `TEMP`/`TMP` the temp dir falls back
/// to an unwritable system folder. Put those back; nothing sarg reads.
pub fn keep_os_env(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    for k in [
        "SystemRoot",
        "SystemDrive",
        "windir",
        "TEMP",
        "TMP",
        "ComSpec",
        "PATHEXT",
    ] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    cmd
}

/// A minimal fixture HTTP server over `std::net::TcpListener`, standing in
/// for the sargineer.com API in tests. Replaces the mocking crate this
/// module used to depend on, which pulled in a whole async executor stack
/// and a parser generator as dev-dependencies purely so this crate could
/// serve canned JSON on localhost — see PLAN.md §8c.
///
/// API shape: deliberately mimics that crate's `server.mock(|when, then|
/// ...)` closure builder (method/path/path_contains/query_param/
/// query_param_exists/header/body_contains on `when`, status/header/body on
/// `then`, `assert_hits`/`assert` on the returned handle) so the 51 call
/// sites across the test files needed no changes beyond dropping its
/// import.
///
/// Real HTTP/1.1 keep-alive: a connection is read for as many requests as
/// the client sends on it (one CLI process can make several calls through
/// `ureq`'s pooled `Agent`) and only ever closed by the peer — a clean EOF
/// on read, never something this server initiates. That side-steps a nasty
/// trap: proactively closing a socket that still has unread bytes in
/// flight makes the kernel answer with RST instead of a plain FIN, which a
/// connection-pooling client reads as "that connection was stale" and
/// silently retries — invisible to the CLI (still exits 0 with the right
/// output) but visible here as a mock's hit count counted twice.
mod fixture {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    pub const GET: &str = "GET";
    pub const POST: &str = "POST";
    pub const PUT: &str = "PUT";

    /// One incoming request, fully read off the wire before matching.
    struct Request {
        method: String,
        path: String,
        /// The request-target exactly as it arrived, query string and all.
        /// Only used to name a request nothing matched: a mismatch on a
        /// `query_param` value is the case where you most need to see the
        /// query, and `path` alone has already dropped it.
        target: String,
        query: Vec<(String, String)>,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    /// Request matcher, built inside the `mock(|when, then| ...)` closure.
    /// Every unset field matches anything; every set field must hold.
    #[derive(Default)]
    pub struct When {
        method: Option<String>,
        path: Option<String>,
        path_contains: Option<String>,
        query_param: Vec<(String, String)>,
        query_param_exists: Vec<String>,
        header: Vec<(String, String)>,
        body_contains: Vec<String>,
    }

    impl When {
        pub fn method(&mut self, m: &str) -> &mut Self {
            self.method = Some(m.to_string());
            self
        }
        /// Exact, and case-SENSITIVE where httpmock's was not. Every path
        /// sarg asks for is lowercase, so nothing here relies on the
        /// difference — but a future mock written in mixed case would.
        pub fn path(&mut self, p: &str) -> &mut Self {
            self.path = Some(p.to_string());
            self
        }
        pub fn path_contains(&mut self, p: &str) -> &mut Self {
            self.path_contains = Some(p.to_string());
            self
        }
        pub fn query_param(&mut self, k: &str, v: &str) -> &mut Self {
            self.query_param.push((k.to_string(), v.to_string()));
            self
        }
        pub fn query_param_exists(&mut self, k: &str) -> &mut Self {
            self.query_param_exists.push(k.to_string());
            self
        }
        /// Exact header match, case-insensitive on the name (used for
        /// `authorization`).
        pub fn header(&mut self, k: &str, v: &str) -> &mut Self {
            self.header.push((k.to_ascii_lowercase(), v.to_string()));
            self
        }
        pub fn body_contains(&mut self, s: &str) -> &mut Self {
            self.body_contains.push(s.to_string());
            self
        }

        fn matches(&self, req: &Request) -> bool {
            if let Some(m) = &self.method {
                if m != &req.method {
                    return false;
                }
            }
            if let Some(p) = &self.path {
                if p != &req.path {
                    return false;
                }
            }
            if let Some(pc) = &self.path_contains {
                if !req.path.contains(pc.as_str()) {
                    return false;
                }
            }
            if !self
                .query_param
                .iter()
                .all(|(k, v)| req.query.iter().any(|(qk, qv)| qk == k && qv == v))
            {
                return false;
            }
            if !self
                .query_param_exists
                .iter()
                .all(|k| req.query.iter().any(|(qk, _)| qk == k))
            {
                return false;
            }
            if !self
                .header
                .iter()
                .all(|(k, v)| req.headers.get(k).map(|got| got == v).unwrap_or(false))
            {
                return false;
            }
            if !self
                .body_contains
                .iter()
                .all(|s| String::from_utf8_lossy(&req.body).contains(s.as_str()))
            {
                return false;
            }
            true
        }
    }

    /// Canned response, built inside the same closure.
    pub struct Then {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Default for Then {
        fn default() -> Self {
            Then {
                status: 200,
                headers: Vec::new(),
                body: Vec::new(),
            }
        }
    }

    impl Then {
        pub fn status(&mut self, s: u16) -> &mut Self {
            self.status = s;
            self
        }
        pub fn header(&mut self, k: &str, v: &str) -> &mut Self {
            self.headers.push((k.to_string(), v.to_string()));
            self
        }
        /// Accepts a `&str` (the common case: a JSON fixture) or bytes (a
        /// binary download fixture).
        pub fn body(&mut self, b: impl AsRef<[u8]>) -> &mut Self {
            self.body = b.as_ref().to_vec();
            self
        }
    }

    struct MockEntry {
        when: When,
        then: Then,
        hits: Arc<AtomicUsize>,
    }

    /// A handle to one registered mock, for `assert_hits` after the run.
    pub struct Mock {
        hits: Arc<AtomicUsize>,
    }

    impl Mock {
        pub fn assert_hits(&self, n: usize) {
            let got = self.hits.load(Ordering::SeqCst);
            assert_eq!(got, n, "expected {n} hit(s), got {got}");
        }

        /// Shorthand for `assert_hits(1)`, used a couple of call sites where
        /// "exactly once" is the whole point of the assertion.
        pub fn assert(&self) {
            self.assert_hits(1);
        }
    }

    struct Inner {
        mocks: Mutex<Vec<MockEntry>>,
        stop: AtomicBool,
        // One handle per accepted connection, so `Drop` can wait for all of
        // them (each is bounded by its own read timeout; in the normal case
        // they've already finished by the time a test drops its Sandbox).
        conns: Mutex<Vec<JoinHandle<()>>>,
    }

    /// The fixture server itself: an accept-loop thread per `Sandbox`, bound
    /// to an OS-assigned port so parallel tests never collide, plus one more
    /// thread per accepted connection. A single accept-loop thread handling
    /// connections one at a time sounds simpler, but under real keep-alive
    /// it head-of-line-blocks: a test that kills its own client mid-response
    /// (`sarg_piped_into_head`, simulating `| head`) leaves a connection open
    /// from the server's point of view until the read timeout, during which
    /// a single-threaded accept loop can't even *accept* the next test's
    /// connection, let alone answer it. A thread per connection means a
    /// stuck one only ever blocks itself.
    pub struct FixtureServer {
        inner: Arc<Inner>,
        port: u16,
        listener_handle: Option<JoinHandle<()>>,
    }

    impl FixtureServer {
        pub fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
            let port = listener.local_addr().unwrap().port();
            let inner = Arc::new(Inner {
                mocks: Mutex::new(Vec::new()),
                stop: AtomicBool::new(false),
                conns: Mutex::new(Vec::new()),
            });
            let thread_inner = inner.clone();
            let listener_handle = thread::spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(s) => {
                            // The `Drop` impl wakes a blocked accept() with a
                            // dummy connection; recognize it and exit rather
                            // than trying to parse a request off it.
                            if thread_inner.stop.load(Ordering::SeqCst) {
                                break;
                            }
                            let conn_inner = thread_inner.clone();
                            let h = thread::spawn(move || handle_connection(s, &conn_inner));
                            thread_inner.conns.lock().unwrap().push(h);
                        }
                        Err(_) => {
                            if thread_inner.stop.load(Ordering::SeqCst) {
                                break;
                            }
                        }
                    }
                }
            });
            FixtureServer {
                inner,
                port,
                listener_handle: Some(listener_handle),
            }
        }

        pub fn base_url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        /// Register a mock. Mocks are checked in registration order and,
        /// once registered, match every matching request (not one-shot) —
        /// `assert_hits(n)` is how a test proves how many times it fired.
        pub fn mock<F>(&self, spec: F) -> Mock
        where
            F: FnOnce(&mut When, &mut Then),
        {
            let mut when = When::default();
            let mut then = Then::default();
            spec(&mut when, &mut then);
            let hits = Arc::new(AtomicUsize::new(0));
            self.inner.mocks.lock().unwrap().push(MockEntry {
                when,
                then,
                hits: hits.clone(),
            });
            Mock { hits }
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.inner.stop.store(true, Ordering::SeqCst);
            // Wake the blocking accept() so the listener thread notices
            // `stop` and exits instead of leaking; the connection itself is
            // never read.
            let woke = TcpStream::connect(("127.0.0.1", self.port)).is_ok();
            if let Some(h) = self.listener_handle.take() {
                // Only join on a ping that landed. If connect failed the
                // listener is still parked in accept() and always will be,
                // so joining would hang the test thread rather than end it;
                // the thread dies with the process instead.
                if woke {
                    let _ = h.join();
                }
            }
            let conns = std::mem::take(&mut *self.inner.conns.lock().unwrap());
            for h in conns {
                let _ = h.join();
            }
        }
    }

    fn handle_connection(stream: TcpStream, inner: &Inner) {
        // Never let a stuck client (or the shutdown wake-up ping) hang this
        // connection's thread, and by extension the suite; also bounds how
        // long we wait for a *next* request on a kept-alive connection.
        //
        // Generous on the read side, because that second job has teeth: one
        // CLI process can pause between two requests on the same pooled
        // connection — `edit_gates_pii_the_same_way_as_new` runs $EDITOR
        // between its GET and its PUT. Time out there and we drop a socket
        // ureq still believes in, and it does not retry a PUT carrying a
        // body: the test fails as "cannot reach", on a slow runner only.
        // Waiting longer costs nothing, because Drop never waits on a
        // connection whose peer has gone — it ends at EOF, not at timeout.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let mut reader = BufReader::new(&stream);
        loop {
            let req = match read_request(&mut reader) {
                Some(r) => r,
                // A clean EOF: the client closed (its process exited after
                // getting what it needed, or died mid-response on purpose —
                // see `sarg_piped_into_head`) or this is the drop-time
                // wake-up ping. Either way we're done — and because we never
                // initiate the close ourselves, we never race a client
                // that's still mid-write with a close-triggered RST.
                None => return,
            };
            let (status, headers, body) = respond(&req, inner);
            let mut w = &stream;
            if write_response(&mut w, status, &headers, &body).is_err() {
                return;
            }
            // Loop back: `ureq`'s Agent may send another request down this
            // same connection (e.g. one CLI process making two calls).
        }
    }

    fn read_request(reader: &mut BufReader<&TcpStream>) -> Option<Request> {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            return None;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();

        let mut headers: HashMap<String, String> = HashMap::new();
        loop {
            let mut hline = String::new();
            let n = reader.read_line(&mut hline).ok()?;
            if n == 0 {
                // EOF mid-headers: the client vanished part-way through
                // sending, so this is a truncated request and not one worth
                // matching or counting a hit for.
                return None;
            }
            let h = hline.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }

        let body_len: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; body_len];
        if body_len > 0 {
            reader.read_exact(&mut body).ok()?;
        }

        let (path, query) = split_target(&target);
        Some(Request {
            method,
            path,
            target,
            query,
            headers,
            body,
        })
    }

    fn split_target(target: &str) -> (String, Vec<(String, String)>) {
        match target.split_once('?') {
            Some((p, q)) => (p.to_string(), parse_query(q)),
            None => (target.to_string(), Vec::new()),
        }
    }

    fn parse_query(qs: &str) -> Vec<(String, String)> {
        qs.split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) => (decode_component(k), decode_component(v)),
                None => (decode_component(pair), String::new()),
            })
            .collect()
    }

    /// `application/x-www-form-urlencoded`-style decode: `+` is a space,
    /// `%XX` is a byte. Matches `client::urlencode`, which is what builds
    /// every query string sarg sends.
    fn decode_component(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b'%' if i + 2 < bytes.len() => {
                    match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                        (Some(hi), Some(lo)) => {
                            out.push((hi << 4) | lo);
                            i += 3;
                        }
                        _ => {
                            out.push(bytes[i]);
                            i += 1;
                        }
                    }
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn hex_val(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }

    /// Find the first mock whose matcher holds and record a hit *before*
    /// writing the response, so a racing `assert_hits` never sees a stale
    /// count. No match is a readable 404, not a hang or a panic.
    fn respond(req: &Request, inner: &Inner) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let mocks = inner.mocks.lock().unwrap();
        for entry in mocks.iter() {
            if entry.when.matches(req) {
                entry.hits.fetch_add(1, Ordering::SeqCst);
                let mut headers = entry.then.headers.clone();
                headers.push((
                    "Content-Length".to_string(),
                    entry.then.body.len().to_string(),
                ));
                return (entry.then.status, headers, entry.then.body.clone());
            }
        }
        drop(mocks);
        let msg = format!(
            "fixture server: no mock matched {} {}",
            req.method, req.target
        );
        let body = msg.into_bytes();
        let headers = vec![
            ("Content-Type".to_string(), "text/plain".to_string()),
            ("Content-Length".to_string(), body.len().to_string()),
        ];
        (404, headers, body)
    }

    fn write_response(
        w: &mut &TcpStream,
        status: u16,
        headers: &[(String, String)],
        body: &[u8],
    ) -> std::io::Result<()> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", status, reason_phrase(status));
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        w.write_all(head.as_bytes())?;
        w.write_all(body)?;
        w.flush()
    }

    fn reason_phrase(status: u16) -> &'static str {
        match status {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            303 => "See Other",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            409 => "Conflict",
            415 => "Unsupported Media Type",
            422 => "Unprocessable Entity",
            500 => "Internal Server Error",
            _ => "Status",
        }
    }
}
