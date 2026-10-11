//! Cloud saving: the packed `.jw` is copied to and from one folder on a Nextcloud server
//! (WebDAV), OneDrive or Google Drive. The local file stays the master copy.
//!
//! Every public function blocks on the network; call them from a worker thread only.

use crate::{
    jw::archive::{self, Stamp},
    platform,
};
use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

const FOLDER: &str = "JustWrite";
const STATE: &str = "cloud.json";
/// The other computer's version, downloaded for a merge.
const INCOMING: &str = "cloud-incoming.jw";
const SECRET: &str = "cloud";
const GRAPH: &str = "https://graph.microsoft.com/v1.0/me/drive/special/approot";
const DRIVE: &str = "https://www.googleapis.com/drive/v3/files";
const DRIVE_UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
/// How long the browser sign-in may take. Other cloud work waits for it.
const SIGN_IN: Duration = Duration::from_secs(180);
const NOT_CONNECTED: &str = "Cloud: connect to a service first (File, Connect to cloud…)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Nextcloud,
    OneDrive,
    GoogleDrive,
}
impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Nextcloud, Provider::OneDrive, Provider::GoogleDrive];
    pub fn name(self) -> &'static str {
        match self {
            Provider::Nextcloud => "Nextcloud",
            Provider::OneDrive => "OneDrive",
            Provider::GoogleDrive => "Google Drive",
        }
    }
}

/// The OAuth app registrations, from the settings.
#[derive(Clone, Default)]
pub struct Ids {
    pub onedrive: String,
    pub google: String,
    pub google_secret: String,
}
impl Ids {
    fn of(&self, provider: Provider) -> Result<(&str, &str), String> {
        let (id, secret, key) = match provider {
            Provider::OneDrive => (&self.onedrive, "", "onedrive_client_id"),
            _ => (&self.google, self.google_secret.as_str(), "google_client_id"),
        };
        if id.is_empty() {
            return Err(format!("Cloud: {} needs an app registration. Put its client id in Settings ({key}).", provider.name()));
        }
        Ok((id, secret))
    }
}

/// The connected service. Its secret (app password or refresh token) is in the system's secret store.
// ponytail: one connection at a time; make this a list when two services are wanted at once.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Account {
    provider: Provider,
    /// Nextcloud: the server address, without a slash at the end.
    #[serde(default)]
    server: String,
    /// Nextcloud: the name to sign in with.
    #[serde(default)]
    login: String,
    /// Nextcloud: the user id in file addresses; it can differ from the login name.
    #[serde(default)]
    user: String,
    /// Google Drive: the id of the cloud folder.
    #[serde(default)]
    folder: String,
}

/// What ties a local `.jw` to its file in the cloud folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub name: String,
    /// The cloud version at the last sync; `None` before the first upload.
    pub tag: Option<String>,
    /// The local file at the last sync.
    pub stamp: Option<Stamp>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    account: Option<Account>,
    /// Linked projects, by the path of the local `.jw`.
    links: BTreeMap<String, Link>,
}
impl State {
    fn load(data: &Path) -> Self {
        fs::read(data.join(STATE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    fn save(&self, data: &Path) -> Result<(), String> {
        let mut temporary = tempfile::Builder::new().prefix(".cloud-").suffix(".tmp").tempfile_in(data).map_err(io)?;
        temporary.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(io)?;
        archive::persist(temporary, &data.join(STATE)).map_err(io)
    }
}
/// One key per file, however its path was spelled when the project was opened.
fn key(jw: &Path) -> String {
    jw.canonicalize().unwrap_or_else(|_| jw.to_owned()).to_string_lossy().into_owned()
}
fn io(error: std::io::Error) -> String {
    format!("Cloud: {error}")
}

// ---- the sync decision -------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Upload,
    Download,
    /// Both sides changed, or the name belongs to a file this link never saw.
    Conflict,
}

/// What to do for a linked project: `local` is the `.jw` on disk, `remote` the version tag in the
/// cloud (`None`: no such file there).
pub fn decide(link: &Link, local: Stamp, remote: Option<&str>) -> Action {
    let Some(remote) = remote else { return Action::Upload };
    let Some(tag) = &link.tag else { return Action::Conflict };
    match (link.stamp != Some(local), remote != tag) {
        (false, false) => Action::Nothing,
        (true, false) => Action::Upload,
        (false, true) => Action::Download,
        (true, true) => Action::Conflict,
    }
}

// ---- small encoders ----------------------------------------------------------------------------

/// Percent-encodes one path segment or query value.
fn enc(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
fn dec(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        match text.get(i + 1..i + 3).filter(|_| bytes[i] == b'%').and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect::<Vec<_>>().join("&")
}
/// Standard base64 with padding, or the URL-safe alphabet without it.
fn base64(bytes: &[u8], url: bool) -> String {
    let table: &[u8; 64] = if url {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(table[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
        if !url {
            out.extend(std::iter::repeat_n('=', 3 - chunk.len()));
        }
    }
    out
}
fn random(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("The system has a random source");
    base64(&buffer, true)
}
/// A WebDAV version tag without its quotes and weak marker.
fn tag(raw: &str) -> String {
    raw.trim().trim_start_matches("W/").trim_matches('"').to_owned()
}

// ---- HTTP --------------------------------------------------------------------------------------

struct Reply {
    status: u16,
    etag: Option<String>,
    location: Option<String>,
    body: Vec<u8>,
}
impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn error(&self, what: &str) -> String {
        format!("Cloud: {what} failed (HTTP {})", self.status)
    }
}

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_global(Some(Duration::from_secs(600)))
        // A dead network must answer soon: on the phone the editor waits for the sync.
        .timeout_resolve(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .user_agent(format!("JustWrite ({})", platform::machine_name()))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .into()
});

fn http(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Reply, String> {
    let mut request = ureq::http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let offline = |e: ureq::Error| format!("Cloud: offline ({e})");
    let invalid = |e: ureq::http::Error| format!("Cloud: {e}");
    let mut response = match body {
        Some(bytes) => AGENT.run(request.body(bytes).map_err(invalid)?),
        None => AGENT.run(request.body(()).map_err(invalid)?),
    }
    .map_err(offline)?;
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let (status, etag, location) = (response.status().as_u16(), header("etag"), header("location"));
    let body = response.body_mut().with_config().limit(1 << 30).read_to_vec().map_err(offline)?;
    Ok(Reply { status, etag, location, body })
}

// ---- signing in --------------------------------------------------------------------------------

struct Endpoints {
    authorize: &'static str,
    token: &'static str,
    scope: &'static str,
    /// More query for the sign-in page.
    extra: &'static str,
}
fn endpoints(provider: Provider) -> Endpoints {
    match provider {
        Provider::OneDrive => Endpoints {
            authorize: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            scope: "Files.ReadWrite.AppFolder offline_access",
            extra: "&prompt=select_account",
        },
        _ => Endpoints {
            authorize: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
            scope: "https://www.googleapis.com/auth/drive.file",
            // Without these Google gives no refresh token.
            extra: "&access_type=offline&prompt=consent",
        },
    }
}

/// The access token and when it stops being valid.
static TOKEN: Mutex<Option<(String, Instant)>> = Mutex::new(None);

// Tests sign in with this and leave the system's secret store alone.
#[cfg(test)]
thread_local!(static TEST_SECRET: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) });

fn stored_secret() -> Result<String, String> {
    #[cfg(test)]
    if let Some(secret) = TEST_SECRET.with(|s| s.borrow().clone()) {
        return Ok(secret);
    }
    let bytes = platform::secret(SECRET)?.get_secret().map_err(|_| "Cloud: not signed in. Connect again.".to_owned())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
fn store_secret(secret: &str) -> Result<(), String> {
    #[cfg(test)]
    if TEST_SECRET.with(|s| s.borrow().is_some()) {
        TEST_SECRET.set(Some(secret.to_owned()));
        return Ok(());
    }
    platform::secret(SECRET)?.set_secret(secret.as_bytes()).map_err(|e| format!("Cloud: the sign-in could not be stored ({e})"))
}

/// Asks the token endpoint for an access token and keeps the refresh token it may send along.
fn token(provider: Provider, ids: &Ids, grant: &[(&str, &str)]) -> Result<String, String> {
    let (id, secret) = ids.of(provider)?;
    let mut pairs = vec![("client_id", id)];
    if !secret.is_empty() {
        pairs.push(("client_secret", secret));
    }
    pairs.extend_from_slice(grant);
    let reply = http("POST", endpoints(provider).token, &[("Content-Type", "application/x-www-form-urlencoded")], Some(form(&pairs).as_bytes()))?;
    let json = reply.json();
    let Some(access) = json["access_token"].as_str() else {
        let why = json["error_description"].as_str().or(json["error"].as_str()).unwrap_or("no token");
        return Err(format!("Cloud: sign-in failed ({}). Connect again.", why.lines().next().unwrap_or(why)));
    };
    if let Some(refresh) = json["refresh_token"].as_str() {
        store_secret(refresh)?;
    }
    let life = json["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60);
    *TOKEN.lock().unwrap() = Some((access.to_owned(), Instant::now() + Duration::from_secs(life)));
    Ok(access.to_owned())
}

/// Signs in through the browser: authorization code with PKCE, answered on a loopback address.
fn sign_in(provider: Provider, ids: &Ids) -> Result<(), String> {
    let (id, _) = ids.of(provider)?;
    let e = endpoints(provider);
    let listener = TcpListener::bind("127.0.0.1:0").map_err(io)?;
    let redirect = format!("http://localhost:{}", listener.local_addr().map_err(io)?.port());
    let (verifier, state) = (random(32), random(16));
    let challenge = base64(&Sha256::digest(verifier.as_bytes()), true);
    let query = form(&[
        ("client_id", id),
        ("response_type", "code"),
        ("redirect_uri", &redirect),
        ("scope", e.scope),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", &state),
    ]);
    if !platform::open_url(&format!("{}?{query}{}", e.authorize, e.extra)) {
        return Err("Cloud: the browser could not be opened".into());
    }
    let code = wait_for_code(&listener, &state)?;
    token(provider, ids, &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("code_verifier", &verifier)])?;
    stored_secret().map(drop).map_err(|_| "Cloud: the service gave no lasting sign-in. Connect again.".into())
}

/// Waits for the browser to come back with `?code=…&state=…`.
fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(io)?;
    let deadline = Instant::now() + SIGN_IN;
    while Instant::now() < deadline {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while !request.contains(&b'\n') && request.len() < 16384 {
            match stream.read(&mut buffer) {
                Ok(n) if n > 0 => request.extend_from_slice(&buffer[..n]),
                _ => break,
            }
        }
        let request = String::from_utf8_lossy(&request);
        let query = request.split_whitespace().nth(1).and_then(|target| target.split_once('?')).map_or("", |(_, q)| q);
        let get = |name: &str| query.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| dec(v));
        let result = match (get("code"), get("error")) {
            (Some(code), _) if get("state").as_deref() == Some(state) => Some(Ok(code)),
            (_, Some(error)) => Some(Err(format!("Cloud: the sign-in was refused ({error})"))),
            // Some other request, such as the one for a page icon.
            _ => None,
        };
        let (status, text) = match &result {
            Some(Ok(_)) => ("200 OK", "JustWrite is connected. You can close this tab."),
            Some(Err(_)) => ("200 OK", "The sign-in did not work. Go back to JustWrite."),
            None => ("404 Not Found", ""),
        };
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
        if let Some(result) = result {
            return result;
        }
    }
    Err("Cloud: the sign-in took too long".into())
}

/// A server address is used over HTTPS only; the app password would travel unprotected otherwise.
fn https(address: &str) -> Result<String, String> {
    let address = address.trim().trim_end_matches('/');
    let address = if address.contains("://") { address.to_owned() } else { format!("https://{address}") };
    if address.len() > 8 && address.starts_with("https://") {
        Ok(address)
    } else {
        Err(format!("Cloud: {address} is not an https:// address"))
    }
}

/// Nextcloud Login Flow v2: the browser signs in, the server hands out an app password.
fn nextcloud_login(server: &str) -> Result<Account, String> {
    let server = https(server)?;
    let start = http("POST", &format!("{server}/index.php/login/v2"), &[], Some(b""))?;
    let json = start.json();
    let (Some(login), Some(endpoint), Some(token)) = (json["login"].as_str(), json["poll"]["endpoint"].as_str(), json["poll"]["token"].as_str()) else {
        return Err(format!("Cloud: {server} did not answer like a Nextcloud server (HTTP {})", start.status));
    };
    let (login, endpoint) = (https(login)?, https(endpoint)?);
    if !platform::open_url(&login) {
        return Err("Cloud: the browser could not be opened".into());
    }
    let body = form(&[("token", token)]);
    let deadline = Instant::now() + SIGN_IN;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(2));
        // 404 until the sign-in is done.
        let Ok(reply) = http("POST", &endpoint, &[("Content-Type", "application/x-www-form-urlencoded")], Some(body.as_bytes())) else { continue };
        let json = reply.json();
        if let (200, Some(server), Some(user), Some(password)) = (reply.status, json["server"].as_str(), json["loginName"].as_str(), json["appPassword"].as_str()) {
            store_secret(password)?;
            return Ok(Account { provider: Provider::Nextcloud, server: https(server)?, login: user.to_owned(), user: String::new(), folder: String::new() });
        }
    }
    Err("Cloud: the sign-in took too long".into())
}

// ---- the three services ------------------------------------------------------------------------

struct Client {
    account: Account,
    /// The `Authorization` header.
    auth: String,
    /// Where the services are. Tests point these at a local server.
    graph: String,
    drive: String,
    drive_upload: String,
}

/// Names and version tags from a WebDAV multistatus reply.
fn multistatus(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let (mut out, mut href, mut etag, mut field) = (Vec::new(), String::new(), String::new(), 0);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                field = match e.local_name().as_ref() {
                    "href" => 1,
                    "getetag" => 2,
                    _ => 0,
                }
            }
            Ok(Event::Text(t)) if field == 1 => href.push_str(&t.xml10_content()),
            Ok(Event::Text(t)) if field == 2 => etag.push_str(&t.xml10_content()),
            Ok(Event::End(e)) => {
                field = 0;
                if e.local_name().as_ref() == "response" {
                    let name = dec(href.trim().trim_end_matches('/').rsplit('/').next().unwrap_or(""));
                    out.push((name, tag(&etag)));
                    href.clear();
                    etag.clear();
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

impl Client {
    fn open(account: &Account, ids: &Ids) -> Result<Self, String> {
        let auth = if account.provider == Provider::Nextcloud {
            format!("Basic {}", base64(format!("{}:{}", account.login, stored_secret()?).as_bytes(), false))
        } else {
            let cached = TOKEN.lock().unwrap().clone().filter(|(_, until)| Instant::now() < *until);
            let access = match cached {
                Some((access, _)) => access,
                None => token(account.provider, ids, &[("grant_type", "refresh_token"), ("refresh_token", &stored_secret()?)])?,
            };
            format!("Bearer {access}")
        };
        Ok(Self { account: account.clone(), auth, graph: GRAPH.into(), drive: DRIVE.into(), drive_upload: DRIVE_UPLOAD.into() })
    }
    fn call(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Reply, String> {
        let mut all = vec![("Authorization", self.auth.as_str())];
        all.extend_from_slice(headers);
        let reply = http(method, url, &all, body)?;
        if reply.status == 401 {
            // The next look asks for a new access token first.
            *TOKEN.lock().unwrap() = None;
            return Err("Cloud: the sign-in was refused. If this stays, connect again.".into());
        }
        Ok(reply)
    }

    /// The WebDAV address of the cloud folder, or of a file in it.
    fn dav(&self, name: &str) -> String {
        let folder = format!("{}/remote.php/dav/files/{}/{FOLDER}", self.account.server, enc(&self.account.user));
        if name.is_empty() { folder } else { format!("{folder}/{}", enc(name)) }
    }
    fn propfind(&self, url: &str, depth: &str) -> Result<Option<Vec<(String, String)>>, String> {
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/></d:prop></d:propfind>"#;
        let reply = self.call("PROPFIND", url, &[("Depth", depth), ("Content-Type", "application/xml")], Some(body.as_bytes()))?;
        match reply.status {
            404 => Ok(None),
            207 => Ok(Some(multistatus(&String::from_utf8_lossy(&reply.body)))),
            _ => Err(reply.error("reading the cloud folder")),
        }
    }

    fn google_query(&self, q: &str, fields: &str) -> Result<Value, String> {
        let reply = self.call("GET", &format!("{}?q={}&fields=files({fields})&orderBy=createdTime&pageSize=200", self.drive, enc(q)), &[], None)?;
        if !reply.ok() {
            return Err(reply.error("reading the cloud folder"));
        }
        Ok(reply.json().get("files").cloned().unwrap_or(Value::Null))
    }
    /// The id of the cloud folder in My Drive; made when it is not there.
    fn google_folder(&self) -> Result<String, String> {
        let kind = "application/vnd.google-apps.folder";
        let found = self.google_query(&format!("name = '{FOLDER}' and mimeType = '{kind}' and trashed = false"), "id")?;
        if let Some(id) = found[0]["id"].as_str() {
            return Ok(id.to_owned());
        }
        let meta = json!({ "name": FOLDER, "mimeType": kind }).to_string();
        let reply = self.call("POST", &format!("{}?fields=id", self.drive), &[("Content-Type", "application/json")], Some(meta.as_bytes()))?;
        reply.json()["id"].as_str().map(str::to_owned).ok_or_else(|| reply.error("making the cloud folder"))
    }
    /// The id and version tag of a file in the cloud folder.
    // ponytail: Drive allows two files with one name; the oldest is used. Store the id in the link if that happens in practice.
    fn google_find(&self, name: &str) -> Result<Option<(String, String)>, String> {
        let name = name.replace('\\', "\\\\").replace('\'', "\\'");
        let files = self.google_query(&format!("'{}' in parents and name = '{name}' and trashed = false", self.account.folder), "id,headRevisionId")?;
        Ok(files[0]["id"].as_str().map(|id| (id.to_owned(), files[0]["headRevisionId"].as_str().unwrap_or("").to_owned())))
    }

    /// The version tag of a file in the cloud folder; `None` when it is not there.
    fn stat(&self, name: &str) -> Result<Option<String>, String> {
        match self.account.provider {
            Provider::Nextcloud => Ok(self.propfind(&self.dav(name), "0")?.and_then(|found| found.into_iter().next()).map(|(_, tag)| tag)),
            Provider::OneDrive => {
                let reply = self.call("GET", &format!("{}:/{}?$select=eTag", self.graph, enc(name)), &[], None)?;
                match reply.status {
                    404 => Ok(None),
                    200 => Ok(reply.json()["eTag"].as_str().map(str::to_owned)),
                    _ => Err(reply.error("checking the cloud file")),
                }
            }
            Provider::GoogleDrive => Ok(self.google_find(name)?.map(|(_, tag)| tag)),
        }
    }
    /// The projects in the cloud folder.
    // ponytail: the first 200 files on OneDrive and Google Drive; follow the next-page links if a folder grows past that.
    fn list(&self) -> Result<Vec<String>, String> {
        let names = |json: &Value| json.as_array().into_iter().flatten().filter_map(|f| f["name"].as_str().map(str::to_owned)).collect::<Vec<_>>();
        let mut found = match self.account.provider {
            Provider::Nextcloud => self.propfind(&self.dav(""), "1")?.unwrap_or_default().into_iter().map(|(name, _)| name).collect(),
            Provider::OneDrive => {
                let reply = self.call("GET", &format!("{}/children?$select=name&$top=200", self.graph), &[], None)?;
                if !reply.ok() {
                    return Err(reply.error("reading the cloud folder"));
                }
                names(&reply.json()["value"])
            }
            Provider::GoogleDrive => names(&self.google_query(&format!("'{}' in parents and trashed = false", self.account.folder), "name")?),
        };
        found.retain(|name| name.ends_with(".jw"));
        found.sort();
        Ok(found)
    }
    fn download(&self, name: &str) -> Result<Vec<u8>, String> {
        let reply = match self.account.provider {
            Provider::Nextcloud => self.call("GET", &self.dav(name), &[], None)?,
            Provider::OneDrive => self.call("GET", &format!("{}:/{}:/content", self.graph, enc(name)), &[], None)?,
            Provider::GoogleDrive => {
                let Some((id, _)) = self.google_find(name)? else { return Err("Cloud: that file is no longer there".into()) };
                self.call("GET", &format!("{}/{id}?alt=media", self.drive), &[], None)?
            }
        };
        if reply.ok() { Ok(reply.body) } else { Err(reply.error("downloading")) }
    }
    /// Uploads a file if the cloud still holds the version `known` (`None`: no file of that name).
    /// Returns the new version tag, or `None` when the cloud changed meanwhile and nothing was written.
    fn upload(&self, name: &str, bytes: &[u8], known: Option<&str>) -> Result<Option<String>, String> {
        let binary = ("Content-Type", "application/octet-stream");
        match self.account.provider {
            Provider::Nextcloud => {
                let condition = match known {
                    Some(tag) => ("If-Match", format!("\"{tag}\"")),
                    None => ("If-None-Match", "*".to_owned()),
                };
                let put = || self.call("PUT", &self.dav(name), &[(condition.0, condition.1.as_str()), binary], Some(bytes));
                let mut reply = put()?;
                if matches!(reply.status, 404 | 409) && known.is_none() {
                    // The folder is not there (it was removed after connecting).
                    self.call("MKCOL", &self.dav(""), &[], None)?;
                    reply = put()?;
                }
                match (reply.status, &reply.etag) {
                    (412, _) => Ok(None),
                    (200..=299, Some(etag)) => Ok(Some(tag(etag))),
                    (200..=299, None) => self.stat(name),
                    _ => Err(reply.error("uploading")),
                }
            }
            Provider::OneDrive => {
                let mut headers = vec![binary];
                let mut url = format!("{}:/{}:/content", self.graph, enc(name));
                match known {
                    Some(tag) => headers.push(("If-Match", tag)),
                    None => url.push_str("?@microsoft.graph.conflictBehavior=fail"),
                }
                let reply = self.call("PUT", &url, &headers, Some(bytes))?;
                match reply.status {
                    409 | 412 => Ok(None),
                    200..=299 => reply.json()["eTag"].as_str().map(|tag| Some(tag.to_owned())).ok_or_else(|| reply.error("uploading")),
                    _ => Err(reply.error("uploading")),
                }
            }
            Provider::GoogleDrive => {
                let found = self.google_find(name)?;
                // ponytail: Drive has no conditional upload. A change between this check and the upload
                // is overwritten; it stays in the file's revision list.
                if found.as_ref().map(|(_, tag)| tag.as_str()) != known {
                    return Ok(None);
                }
                let (json, fields) = (("Content-Type", "application/json"), "uploadType=resumable&fields=headRevisionId");
                let start = match &found {
                    Some((id, _)) => self.call("PATCH", &format!("{}/{id}?{fields}", self.drive_upload), &[json], Some(b"{}"))?,
                    None => {
                        let meta = json!({ "name": name, "parents": [self.account.folder] }).to_string();
                        self.call("POST", &format!("{}?{fields}", self.drive_upload), &[json], Some(meta.as_bytes()))?
                    }
                };
                let Some(session) = start.location.as_deref().filter(|_| start.ok()) else { return Err(start.error("uploading")) };
                let reply = self.call("PUT", session, &[binary], Some(bytes))?;
                if !reply.ok() {
                    return Err(reply.error("uploading"));
                }
                match reply.json()["headRevisionId"].as_str() {
                    Some(tag) => Ok(Some(tag.to_owned())),
                    None => self.stat(name),
                }
            }
        }
    }
}

// ---- what the application calls ----------------------------------------------------------------

/// Connects to a service; the browser does the sign-in. This replaces the current connection.
pub fn connect(data: &Path, provider: Provider, server: &str, ids: &Ids) -> Result<String, String> {
    // Checked first: a mistake here must not cost the connection that works.
    match provider {
        Provider::Nextcloud => drop(https(server)?),
        _ => drop(ids.of(provider)?),
    }
    disconnect(data);
    let account = match provider {
        Provider::Nextcloud => nextcloud_login(server)?,
        _ => {
            sign_in(provider, ids)?;
            Account { provider, server: String::new(), login: String::new(), user: String::new(), folder: String::new() }
        }
    };
    establish(data, account, ids)
}

/// Connects to Nextcloud with an app password made in its web settings, where the browser sign-in
/// cannot be waited for (iOS stops the app while the browser is in front). The password is the
/// user's: it is not taken back on the server when the connection is replaced.
pub fn connect_with_password(data: &Path, server: &str, login: &str, password: &str) -> Result<String, String> {
    let account = Account { provider: Provider::Nextcloud, server: https(server)?, login: login.trim().to_owned(), user: String::new(), folder: String::new() };
    store_secret(password.trim())?;
    establish(data, account, &Ids::default())
}

/// Checks the sign-in, makes the cloud folder and records the connection, with no links.
fn establish(data: &Path, mut account: Account, ids: &Ids) -> Result<String, String> {
    let provider = account.provider;
    let mut client = Client::open(&account, ids)?;
    match provider {
        Provider::Nextcloud => {
            let me = client.call("GET", &format!("{}/ocs/v2.php/cloud/user?format=json", account.server), &[("OCS-APIRequest", "true")], None)?;
            account.user = me.json()["ocs"]["data"]["id"].as_str().ok_or_else(|| me.error("reading the account"))?.to_owned();
            client.account = account.clone();
            // 405: the folder is there already.
            let reply = client.call("MKCOL", &client.dav(""), &[], None)?;
            if !reply.ok() && reply.status != 405 {
                return Err(reply.error("making the cloud folder"));
            }
        }
        // Asking for the app folder makes it.
        Provider::OneDrive => {
            let reply = client.call("GET", &client.graph, &[], None)?;
            if !reply.ok() {
                return Err(reply.error("opening the app folder"));
            }
        }
        Provider::GoogleDrive => account.folder = client.google_folder()?,
    }
    State { account: Some(account), links: BTreeMap::new() }.save(data)?;
    Ok(format!("Cloud: connected to {}", provider.name()))
}

/// Forgets the connection and every link, and takes back the sign-in where the service allows it.
pub fn disconnect(data: &Path) -> String {
    if let Some(account) = State::load(data).account {
        match account.provider {
            Provider::Nextcloud => {
                if let Ok(client) = Client::open(&account, &Ids::default()) {
                    let _ = client.call("DELETE", &format!("{}/ocs/v2.php/core/apppassword", account.server), &[("OCS-APIRequest", "true")], None);
                }
            }
            Provider::GoogleDrive => {
                if let Ok(refresh) = stored_secret() {
                    let _ = http("POST", "https://oauth2.googleapis.com/revoke", &[("Content-Type", "application/x-www-form-urlencoded")], Some(form(&[("token", &refresh)]).as_bytes()));
                }
            }
            // Microsoft has no call for this; the sign-in ends when it is not used.
            Provider::OneDrive => {}
        }
    }
    if let Ok(entry) = platform::secret(SECRET) {
        let _ = entry.delete_credential();
    }
    *TOKEN.lock().unwrap() = None;
    let _ = State::default().save(data);
    "Cloud: disconnected".into()
}

fn connected(data: &Path, ids: &Ids) -> Result<(State, Client), String> {
    let state = State::load(data);
    let account = state.account.clone().ok_or(NOT_CONNECTED)?;
    let client = Client::open(&account, ids)?;
    Ok((state, client))
}

/// The projects in the cloud folder.
pub fn list(data: &Path, ids: &Ids) -> Result<Vec<String>, String> {
    connected(data, ids)?.1.list()
}

/// Downloads a project from the cloud folder to `dest` and links the two.
pub fn fetch(data: &Path, ids: &Ids, name: &str, dest: &Path) -> Result<(), String> {
    let (mut state, client) = connected(data, ids)?;
    let tag = client.stat(name)?.ok_or("Cloud: that file is no longer there")?;
    let stamp = replace(dest, &client.download(name)?, None)?;
    state.links.insert(key(dest), Link { name: name.to_owned(), tag: Some(tag), stamp });
    state.save(data)
}

/// Links a local project to a file of the same name in the cloud folder; the next sync uploads it.
pub fn link(data: &Path, jw: &Path) -> Result<(), String> {
    let mut state = State::load(data);
    if state.account.is_none() {
        return Err(NOT_CONNECTED.into());
    }
    let name = jw.file_name().ok_or("Cloud: the project has no file name")?.to_string_lossy().into_owned();
    state.links.entry(key(jw)).or_insert(Link { name, tag: None, stamp: None });
    state.save(data)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Synced {
    NotLinked,
    Nothing,
    Uploaded,
    /// The local `.jw` was replaced by the cloud version.
    Downloaded,
    /// Both sides changed. The cloud version is in this file, with this version tag, for the
    /// application to merge through the project history; `accept` then lets the result go up.
    Incoming(PathBuf, String),
    /// The cloud file of this name is another project. The local one went up under this new
    /// name and is linked to it.
    Conflict(String),
}

/// Brings a linked project and its cloud file in line. Never overwrites a version it did not see.
pub fn sync(data: &Path, ids: &Ids, jw: &Path) -> Result<Synced, String> {
    let state = State::load(data);
    let (Some(account), Some(mut link)) = (state.account.clone(), state.links.get(&key(jw)).cloned()) else { return Ok(Synced::NotLinked) };
    let outcome = sync_link(&Client::open(&account, ids)?, &mut link, jw, &data.join(INCOMING))?;
    if !matches!(outcome, Synced::Nothing | Synced::Incoming(..)) {
        // Read again: the transfer can take minutes and the file may have changed meanwhile.
        let mut state = State::load(data);
        state.links.insert(key(jw), link);
        state.save(data)?;
    }
    Ok(outcome)
}

/// The cloud version `tag` was merged into the local project: the next upload may replace it.
pub fn accept(data: &Path, jw: &Path, tag: String) -> Result<(), String> {
    let mut state = State::load(data);
    let link = state.links.get_mut(&key(jw)).ok_or("Cloud: the project is no longer linked")?;
    link.tag = Some(tag);
    state.save(data)
}

fn sync_link(client: &Client, link: &mut Link, jw: &Path, incoming: &Path) -> Result<Synced, String> {
    let remote = client.stat(&link.name)?;
    let local = archive::stamp(jw).map_err(io)?;
    // `None` from a transfer: the other side changed meanwhile. The next round decides again.
    match decide(link, local, remote.as_deref()) {
        Action::Nothing => Ok(Synced::Nothing),
        Action::Upload => {
            let (bytes, stamp) = read(jw)?;
            let Some(tag) = client.upload(&link.name, &bytes, remote.as_deref())? else { return Ok(Synced::Nothing) };
            (link.tag, link.stamp) = (Some(tag), Some(stamp));
            Ok(Synced::Uploaded)
        }
        Action::Download => {
            let Some(stamp) = replace(jw, &client.download(&link.name)?, Some(local))? else { return Ok(Synced::Nothing) };
            (link.tag, link.stamp) = (remote, Some(stamp));
            Ok(Synced::Downloaded)
        }
        Action::Conflict => {
            // The same project from another computer is merged, not set beside this one.
            fs::write(incoming, client.download(&link.name)?).map_err(io)?;
            let id = |path: &Path| archive::read_manifest(path).ok().map(|manifest| manifest.id);
            if let (Some(tag), true) = (&remote, id(incoming).is_some() && id(incoming) == id(jw)) {
                return Ok(Synced::Incoming(incoming.to_owned(), tag.clone()));
            }
            let _ = fs::remove_file(incoming);
            let (bytes, stamp) = read(jw)?;
            let name = format!(
                "{} (conflict, {}, {}).jw",
                link.name.trim_end_matches(".jw"),
                platform::machine_name(),
                jiff::Zoned::now().strftime("%Y-%m-%d %H.%M.%S")
            );
            let Some(tag) = client.upload(&name, &bytes, None)? else { return Ok(Synced::Nothing) };
            *link = Link { name: name.clone(), tag: Some(tag), stamp: Some(stamp) };
            Ok(Synced::Conflict(name))
        }
    }
}

/// The whole `.jw` and its stamp, not torn by a pack.
fn read(jw: &Path) -> Result<(Vec<u8>, Stamp), String> {
    let _writing = archive::WRITING.lock();
    Ok((fs::read(jw).map_err(io)?, archive::stamp(jw).map_err(io)?))
}

/// Puts downloaded bytes in place of `jw` in one step. With `expect`, only while the file is still
/// that version; `None` is returned when it is not.
fn replace(jw: &Path, bytes: &[u8], expect: Option<Stamp>) -> Result<Option<Stamp>, String> {
    let parent = jw.parent().ok_or("Cloud: the project has no folder")?;
    let mut temporary = tempfile::Builder::new().prefix(".cloud-").suffix(".tmp").tempfile_in(parent).map_err(io)?;
    temporary.write_all(bytes).and_then(|()| temporary.as_file().sync_all()).map_err(io)?;
    archive::read_manifest(temporary.path()).map_err(|e| format!("Cloud: the downloaded file is not a project ({e})"))?;
    let _writing = archive::WRITING.lock();
    if expect.is_some() && archive::stamp(jw).ok() != expect {
        return Ok(None);
    }
    archive::persist(temporary, jw).map_err(io)?;
    archive::stamp(jw).map(Some).map_err(io)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::jw::{Manifest, workdir};
    use std::io::{BufRead, BufReader};

    #[test]
    fn decide_covers_the_table() {
        let stamp = Stamp { len: 10, mtime_ms: 1 };
        let other = Stamp { len: 11, mtime_ms: 2 };
        let synced = Link { name: "a.jw".into(), tag: Some("t1".into()), stamp: Some(stamp) };
        assert_eq!(decide(&synced, stamp, Some("t1")), Action::Nothing);
        assert_eq!(decide(&synced, other, Some("t1")), Action::Upload);
        assert_eq!(decide(&synced, stamp, Some("t2")), Action::Download);
        assert_eq!(decide(&synced, other, Some("t2")), Action::Conflict);
        // The cloud file was deleted: it goes up again.
        assert_eq!(decide(&synced, stamp, None), Action::Upload);
        let fresh = Link { name: "a.jw".into(), tag: None, stamp: None };
        assert_eq!(decide(&fresh, stamp, None), Action::Upload);
        // The name is taken by a file this link never saw.
        assert_eq!(decide(&fresh, stamp, Some("t1")), Action::Conflict);
    }

    #[test]
    fn encoders_match_the_standards() {
        // RFC 7636, appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(base64(&Sha256::digest(verifier.as_bytes()), true), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(base64(b"user:pass", false), "dXNlcjpwYXNz");
        assert_eq!(base64(b"ab", false), "YWI=");
        assert_eq!(base64(b"a", false), "YQ==");
        assert_eq!(enc("My novel (1).jw"), "My%20novel%20%281%29.jw");
        assert_eq!(dec("My%20novel%20%281%29.jw"), "My novel (1).jw");
        assert_eq!(dec("100%"), "100%");
        assert_eq!(random(32).len(), 43);
        assert!(https("http://nas.local").is_err());
        assert_eq!(https("cloud.example.org/").unwrap(), "https://cloud.example.org");
    }

    #[test]
    fn the_sign_in_answer_must_carry_our_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let browser = std::thread::spawn(move || {
            let mut replies = Vec::new();
            for target in ["/favicon.ico", "/?code=stolen&state=other", "/?state=mine&code=a%2Fb"] {
                let mut stream = std::net::TcpStream::connect(address).unwrap();
                write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut reply = String::new();
                stream.read_to_string(&mut reply).unwrap();
                replies.push(reply);
            }
            replies
        });
        assert_eq!(wait_for_code(&listener, "mine").unwrap(), "a/b");
        let replies = browser.join().unwrap();
        assert!(replies[0].starts_with("HTTP/1.1 404") && replies[1].starts_with("HTTP/1.1 404") && replies[2].contains("connected"));
    }

    /// Needs the network and this computer's secret store: `cargo test cloud -- --ignored`.
    #[test]
    #[ignore]
    fn this_computer_reaches_the_services_and_keeps_a_secret() {
        let entry = platform::secret("cloud-test").unwrap();
        entry.set_secret("s3cret".as_bytes()).unwrap();
        assert_eq!(platform::secret("cloud-test").unwrap().get_secret().unwrap(), b"s3cret");
        entry.delete_credential().unwrap();
        // No sign-in: each service must answer over TLS and refuse.
        assert_eq!(http("GET", GRAPH, &[], None).unwrap().status, 401);
        assert!(matches!(http("GET", DRIVE, &[], None).unwrap().status, 401 | 403));
        let ids = Ids { onedrive: "00000000-0000-0000-0000-000000000000".into(), google: "x.apps.googleusercontent.com".into(), google_secret: String::new() };
        for provider in [Provider::OneDrive, Provider::GoogleDrive] {
            let error = token(provider, &ids, &[("grant_type", "refresh_token"), ("refresh_token", "bogus")]).unwrap_err();
            assert!(error.starts_with("Cloud: sign-in failed ("), "{error}");
        }
    }

    /// A local server that speaks enough WebDAV, Microsoft Graph and Google Drive for the sync
    /// rules. Files are kept in memory, with a counter as the version.
    pub(crate) fn server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let base = address.clone();
        std::thread::spawn(move || {
            let mut files: BTreeMap<String, (Vec<u8>, u32)> = BTreeMap::new();
            let (mut version, mut folder) = (0, false);
            let dav_tag = |v: u32| format!("\"v{v}\"");
            let graph_tag = |v: u32| format!("\"{{F00}},{v}\"");
            let json = |value: Value| value.to_string().into_bytes();
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, target) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                let mut headers = BTreeMap::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    let Some((name, value)) = line.trim().split_once(':') else { break };
                    headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
                }
                let mut body = vec![0u8; headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0)];
                reader.read_exact(&mut body).unwrap();
                assert_eq!(headers.get("authorization").map(String::as_str), Some("Basic YW5uOnRlc3Q="));
                let missing = ("404 Not Found", String::new(), Vec::new());
                let refused = ("412 Precondition Failed", String::new(), Vec::new());
                let (status, header, reply): (&str, String, Vec<u8>) = if let Some(rest) = path.strip_prefix("/remote.php/dav/files/ann/JustWrite") {
                    let name = dec(rest.trim_start_matches('/'));
                    let existing = files.get(&name).map(|(_, v)| *v);
                    let row = |href: &str, tag: &str| format!("<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:getetag>{tag}</d:getetag></d:prop></d:propstat></d:response>");
                    let multistatus = |rows: String| ("207 Multi-Status", String::new(), format!("<d:multistatus xmlns:d=\"DAV:\">{rows}</d:multistatus>").into_bytes());
                    match (method.as_str(), existing) {
                        ("MKCOL", _) => ("201 Created", String::new(), Vec::new()),
                        ("PROPFIND", _) if name.is_empty() => {
                            multistatus(row(&format!("{path}/"), "dir") + &files.iter().map(|(n, (_, v))| row(&format!("{path}/{}", enc(n)), &format!("&quot;v{v}&quot;"))).collect::<String>())
                        }
                        ("PROPFIND", Some(v)) => multistatus(row(path, &dav_tag(v))),
                        ("GET", Some(v)) => ("200 OK", format!("ETag: {}\r\n", dav_tag(v)), files[&name].0.clone()),
                        ("PUT", _) => {
                            let allowed = match (headers.get("if-match"), headers.get("if-none-match")) {
                                (Some(wanted), _) => existing.map(dav_tag).as_ref() == Some(wanted),
                                (None, Some(_)) => existing.is_none(),
                                (None, None) => true,
                            };
                            if allowed {
                                version += 1;
                                files.insert(name, (body, version));
                                ("201 Created", format!("ETag: {}\r\n", dav_tag(version)), Vec::new())
                            } else {
                                refused
                            }
                        }
                        _ => missing,
                    }
                } else if path == "/graph/children" {
                    ("200 OK", String::new(), json(json!({ "value": files.keys().map(|n| json!({ "name": n })).collect::<Vec<_>>() })))
                } else if let Some(rest) = path.strip_prefix("/graph:/") {
                    let name = dec(rest.trim_end_matches(":/content"));
                    let existing = files.get(&name).map(|(_, v)| *v);
                    match (method.as_str(), rest.ends_with(":/content"), existing) {
                        ("GET", false, Some(v)) => ("200 OK", String::new(), json(json!({ "eTag": graph_tag(v) }))),
                        ("GET", true, Some(_)) => ("200 OK", String::new(), files[&name].0.clone()),
                        ("PUT", true, _) => match headers.get("if-match") {
                            Some(wanted) if existing.map(graph_tag).as_ref() != Some(wanted) => refused,
                            None if existing.is_some() && query.contains("conflictBehavior=fail") => ("409 Conflict", String::new(), Vec::new()),
                            _ => {
                                version += 1;
                                files.insert(name, (body, version));
                                ("201 Created", String::new(), json(json!({ "eTag": graph_tag(version) })))
                            }
                        },
                        _ => missing,
                    }
                } else if path == "/drive" && method == "GET" {
                    let q = query.split('&').find_map(|p| p.strip_prefix("q=")).map(dec).unwrap_or_default();
                    let found: Vec<Value> = if q.contains("mimeType") {
                        if folder { vec![json!({ "id": "folder1" })] } else { Vec::new() }
                    } else {
                        assert!(q.starts_with("'folder1' in parents"), "{q}");
                        let wanted = q.split_once("name = '").map(|(_, rest)| rest.trim_end_matches("' and trashed = false").replace("\\'", "'").replace("\\\\", "\\"));
                        files
                            .iter()
                            .filter(|(n, _)| wanted.as_ref().is_none_or(|w| w == *n))
                            .map(|(n, (_, v))| json!({ "id": enc(n), "name": n, "headRevisionId": format!("r{v}") }))
                            .collect()
                    };
                    ("200 OK", String::new(), json(json!({ "files": found })))
                } else if path == "/drive" {
                    folder = true;
                    ("200 OK", String::new(), json(json!({ "id": "folder1" })))
                } else if let Some(id) = path.strip_prefix("/drive/") {
                    files.get(&dec(id)).map_or(missing, |(bytes, _)| ("200 OK", String::new(), bytes.clone()))
                } else if path.starts_with("/upload") {
                    // The start of a resumable upload: the answer says where the bytes go.
                    let id = match path.strip_prefix("/upload/") {
                        Some(id) => id.to_owned(),
                        None => {
                            let meta: Value = serde_json::from_slice(&body).unwrap();
                            assert_eq!(meta["parents"][0], "folder1");
                            enc(meta["name"].as_str().unwrap())
                        }
                    };
                    ("200 OK", format!("Location: {base}/session/{id}\r\n"), Vec::new())
                } else if let Some(id) = path.strip_prefix("/session/") {
                    version += 1;
                    files.insert(dec(id), (body, version));
                    ("200 OK", String::new(), json(json!({ "headRevisionId": format!("r{version}") })))
                } else {
                    missing
                };
                write!(stream, "HTTP/1.1 {status}\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n", reply.len()).unwrap();
                stream.write_all(&reply).unwrap();
            }
        });
        address
    }

    /// From here on this thread keeps its sign-in in memory.
    pub(crate) fn no_secret_store() {
        TEST_SECRET.set(Some(String::new()));
    }
    /// Signs this thread in to the Nextcloud of the test server, with `data` as the data folder.
    pub(crate) fn sign_in_to(base: &str, data: &Path) {
        TEST_SECRET.set(Some("test".into()));
        let account = Account { provider: Provider::Nextcloud, server: base.into(), login: "ann".into(), user: "ann".into(), folder: String::new() };
        State { account: Some(account), links: BTreeMap::new() }.save(data).unwrap();
    }

    /// A packed project with one chapter.
    fn project(data: &Path, jw: &Path, text: &str) -> workdir::WorkDir {
        let work = workdir::create(data, &Manifest::new("Novel")).unwrap();
        work.write("chapters/one.txt", text.as_bytes()).unwrap();
        work.pack(jw, false).unwrap();
        work
    }

    #[test]
    fn a_project_goes_up_comes_down_and_never_overwrites() {
        for provider in Provider::ALL {
            let base = server();
            let mut client = Client {
                account: Account { provider, server: base.clone(), login: String::new(), user: "ann".into(), folder: String::new() },
                auth: "Basic YW5uOnRlc3Q=".into(),
                graph: format!("{base}/graph"),
                drive: format!("{base}/drive"),
                drive_upload: format!("{base}/upload"),
            };
            if provider == Provider::GoogleDrive {
                // Made at the first look, found at the second.
                client.account.folder = client.google_folder().unwrap();
                assert_eq!(client.google_folder().unwrap(), "folder1");
            }
            assert_eq!(client.list().unwrap(), [""; 0], "{provider:?}");
            scenario(&client, "Ann's novel.jw");
        }
    }

    /// The sync rules against one service. Leaves `cloud_name` and the conflict copy it returns
    /// in the cloud folder.
    fn scenario(client: &Client, cloud_name: &str) -> String {
        let provider = client.account.provider;
        let (data, here, there, my_data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let jw = here.path().join(cloud_name);
        let incoming = here.path().join("incoming.jw");
        let work = project(data.path(), &jw, "first draft");
        let mut link = Link { name: cloud_name.into(), tag: None, stamp: None };

        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Uploaded, "{provider:?}");
        let first = link.tag.clone().unwrap();
        assert_eq!(client.stat(cloud_name).unwrap(), Some(first.clone()));
        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Nothing);
        assert!(client.list().unwrap().iter().any(|name| name == cloud_name));

        // A local edit goes up.
        std::thread::sleep(Duration::from_millis(15));
        work.write("chapters/one.txt", b"second draft").unwrap();
        work.pack(&jw, false).unwrap();
        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Uploaded);
        assert_eq!(client.download(cloud_name).unwrap(), fs::read(&jw).unwrap());

        // An upload that names an old version, or no version, is refused.
        assert_eq!(client.upload(cloud_name, b"x", Some(&first)).unwrap(), None, "{provider:?}");
        assert_eq!(client.upload(cloud_name, b"x", None).unwrap(), None, "{provider:?}");
        assert_eq!(client.download(cloud_name).unwrap(), fs::read(&jw).unwrap());

        // Another computer uploads; with no local change its version comes down.
        let (theirs, their_data) = (there.path().join("theirs.jw"), tempfile::tempdir().unwrap());
        let their_work = project(their_data.path(), &theirs, "edited elsewhere");
        let their_bytes = fs::read(&theirs).unwrap();
        assert!(client.upload(cloud_name, &their_bytes, link.tag.as_deref()).unwrap().is_some());
        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Downloaded, "{provider:?}");
        assert_eq!(fs::read(&jw).unwrap(), their_bytes);
        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Nothing);

        // Both sides change the same project: the cloud version is handed over for a merge,
        // and what the merge made goes up only when that version was accepted.
        std::thread::sleep(Duration::from_millis(15));
        their_work.write("chapters/one.txt", b"my edit of it").unwrap();
        their_work.pack(&jw, true).unwrap();
        their_work.write("chapters/one.txt", b"edited again elsewhere").unwrap();
        their_work.pack(&theirs, true).unwrap();
        let newer = fs::read(&theirs).unwrap();
        let seen = link.clone();
        assert!(client.upload(cloud_name, &newer, link.tag.as_deref()).unwrap().is_some());
        let Synced::Incoming(file, tag) = sync_link(&client, &mut link, &jw, &incoming).unwrap() else { panic!("{provider:?}: a merge expected") };
        assert_eq!((fs::read(&file).unwrap(), Some(&tag), &link), (newer.clone(), client.stat(cloud_name).unwrap().as_ref(), &seen));
        assert_eq!(client.download(cloud_name).unwrap(), newer);
        link.tag = Some(tag);
        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Uploaded);
        assert_eq!(client.download(cloud_name).unwrap(), fs::read(&jw).unwrap());

        // The name holds another project: that one stays, the local one goes up under a new name.
        assert!(client.upload(cloud_name, &their_bytes, link.tag.as_deref()).unwrap().is_some());
        let mine = project(my_data.path(), &there.path().join("mine.jw"), "my edit");
        std::thread::sleep(Duration::from_millis(15));
        mine.pack(&jw, true).unwrap();
        let Synced::Conflict(name) = sync_link(&client, &mut link, &jw, &incoming).unwrap() else { panic!("conflict expected") };
        assert!(name.starts_with(&format!("{} (conflict, ", cloud_name.trim_end_matches(".jw"))) && link.name == name);
        assert_eq!(client.download(cloud_name).unwrap(), their_bytes);
        assert_eq!(client.download(&name).unwrap(), fs::read(&jw).unwrap());
        assert_eq!(sync_link(&client, &mut link, &jw, &incoming).unwrap(), Synced::Nothing);
        assert!(client.list().unwrap().contains(&name));

        // A download that is not a project is refused and the local file is kept.
        assert!(client.upload(&name, b"not a zip", link.tag.as_deref()).unwrap().is_some());
        let before = fs::read(&jw).unwrap();
        assert!(sync_link(&client, &mut link, &jw, &incoming).is_err());
        assert_eq!(fs::read(&jw).unwrap(), before);
        name
    }

    /// The same rules against a real Nextcloud server. The browser opens for the sign-in; the
    /// test removes its files and the sign-in afterwards.
    /// `JUSTWRITE_TEST_NEXTCLOUD=https://… cargo test --lib a_real_nextcloud -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_real_nextcloud_follows_the_rules() {
        let server = std::env::var("JUSTWRITE_TEST_NEXTCLOUD").expect("JUSTWRITE_TEST_NEXTCLOUD holds the server address");
        let data = tempfile::tempdir().unwrap();
        println!("{}", connect(data.path(), Provider::Nextcloud, &server, &Ids::default()).unwrap());
        let (_, client) = connected(data.path(), &Ids::default()).unwrap();
        println!("signed in as {} (user id {}) at {}", client.account.login, client.account.user, client.account.server);
        let stem = format!("JustWrite test {}", random(4));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scenario(&client, &format!("{stem}.jw"))));
        for name in client.list().unwrap().iter().filter(|name| name.starts_with(&stem)) {
            println!("removing {name}: HTTP {}", client.call("DELETE", &client.dav(name), &[], None).unwrap().status);
        }
        println!("{}", disconnect(data.path()));
        assert!(stored_secret().is_err());
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }
}
