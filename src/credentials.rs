//! The credential store: best-mcp's own file, `~/.best-mcp/credentials.json`, read and written entry for entry so a
//! person and their agent share sign-ins. One entry per connection name: `tenantId`, `apiKey`, `authType`,
//! `authHeader`, `issuedAt`, `manifest` (the address signed in at) and `endpoints` (service id → endpoint).
//! Entries this program did not write, and members it does not know, pass through untouched. The credential is
//! never printed: it leaves this module only as a request header.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

/// The variable best-mcp honours for the file's location; the same variable moves both programs.
pub const FILE_VARIABLE: &str = "BEST_MCP_CREDENTIALS_FILE";

/// A connection name as best-mcp accepts it: letters, digits, '.', '_' and '-', up to 64, starting with a letter
/// or digit.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The connection's name as the device request carries it (SPEC Agent Registration): one word of lowercase
/// letters, digits and hyphens made from the name the person gave.
pub fn connection_word(name: &str) -> String {
    let mut word = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            word.push(c);
        } else if !word.ends_with('-') && !word.is_empty() {
            word.push('-');
        }
    }
    let word: String = word.trim_end_matches('-').chars().take(64).collect();
    if word.is_empty() {
        "best".into()
    } else {
        word
    }
}

/// A credential, shown to nobody.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The value, for the one place that puts it in a header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<credential>")
    }
}

/// How the credential travels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// A public surface: nothing is sent.
    None,
    /// `Authorization: Bearer <credential>`.
    Bearer(Secret),
    /// `<header>: <credential>`.
    ApiKey { header: String, key: Secret },
}

/// One signed-in connection.
#[derive(Debug, Clone)]
pub struct Entry {
    pub tenant_id: Option<String>,
    pub auth: Auth,
    pub issued_at: Option<String>,
    /// The manifest address the sign-in started at.
    pub manifest: Option<String>,
    /// Service id → the endpoint the credential opens, in the file's order.
    pub endpoints: Vec<(String, String)>,
}

impl Entry {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let text = |k: &str| object.get(k).and_then(Value::as_str).map(str::to_owned);
        let api_key = text("apiKey");
        let auth_type = text("authType").unwrap_or_else(|| "apikey".into());
        let auth = match (auth_type.to_ascii_lowercase().as_str(), api_key) {
            ("none", _) | (_, None) => Auth::None,
            ("bearer", Some(key)) => Auth::Bearer(Secret(key)),
            (_, Some(key)) => Auth::ApiKey {
                header: text("authHeader").unwrap_or_else(|| "X-Api-Key".into()),
                key: Secret(key),
            },
        };
        let endpoints = object
            .get("endpoints")
            .and_then(Value::as_object)
            .map(|e| {
                e.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.trim_end_matches('/').to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            tenant_id: text("tenantId"),
            auth,
            issued_at: text("issuedAt"),
            manifest: text("manifest"),
            endpoints,
        })
    }

    /// The entry as best-mcp writes it, member for member, in its order.
    fn to_value(&self) -> Value {
        let mut object = Map::new();
        if let Some(t) = &self.tenant_id {
            object.insert("tenantId".into(), Value::String(t.clone()));
        }
        match &self.auth {
            Auth::None => {
                object.insert("authType".into(), Value::String("none".into()));
            }
            Auth::Bearer(key) => {
                object.insert("apiKey".into(), Value::String(key.0.clone()));
                object.insert("authType".into(), Value::String("bearer".into()));
            }
            Auth::ApiKey { header, key } => {
                object.insert("apiKey".into(), Value::String(key.0.clone()));
                object.insert("authType".into(), Value::String("apikey".into()));
                object.insert("authHeader".into(), Value::String(header.clone()));
            }
        }
        if let Some(t) = &self.issued_at {
            object.insert("issuedAt".into(), Value::String(t.clone()));
        }
        if let Some(m) = &self.manifest {
            object.insert("manifest".into(), Value::String(m.clone()));
        }
        let endpoints: Map<String, Value> = self
            .endpoints
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        object.insert("endpoints".into(), Value::Object(endpoints));
        Value::Object(object)
    }

    /// The host the sign-in resolved to, for the person's eyes.
    pub fn host(&self) -> String {
        self.manifest
            .as_deref()
            .and_then(|m| url::Url::parse(m).ok())
            .and_then(|u| {
                u.host_str().map(|h| match u.port() {
                    Some(p) => format!("{h}:{p}"),
                    None => h.to_owned(),
                })
            })
            .or_else(|| {
                self.endpoints
                    .first()
                    .and_then(|(_, e)| url::Url::parse(e).ok())
                    .and_then(|u| u.host_str().map(str::to_owned))
            })
            .unwrap_or_default()
    }

    /// The endpoint a name opens: the first service's, or the one named after a slash (`name/io.x.y`).
    pub fn endpoint(&self, service: Option<&str>) -> Result<(String, String)> {
        match service {
            Some(id) => self.endpoints.iter().find(|(k, _)| k == id).cloned().with_context(|| {
                format!(
                    "no service '{id}' on this connection; it has: {}",
                    self.endpoints
                        .iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }),
            None => self
                .endpoints
                .first()
                .cloned()
                .context("the connection names no endpoint"),
        }
    }
}

/// The file, loaded whole so that what is not ours is written back as it was.
pub struct Store {
    path: PathBuf,
    entries: Map<String, Value>,
}

impl Store {
    /// `BEST_MCP_CREDENTIALS_FILE`, else `~/.best-mcp/credentials.json`. A missing file is an empty store.
    pub fn open() -> Result<Self> {
        let path = match std::env::var_os(FILE_VARIABLE) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => dirs::home_dir()
                .with_context(|| format!("no home directory; set {FILE_VARIABLE}"))?
                .join(".best-mcp")
                .join("credentials.json"),
        };
        let entries = match fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => Map::new(),
            Ok(text) => {
                match serde_json::from_str::<Value>(&text).with_context(|| format!("{} is not JSON", path.display()))? {
                    Value::Object(m) => m,
                    _ => bail!("{} is not a JSON object of connections", path.display()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, entries })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, name: &str) -> Option<Entry> {
        self.entries.get(name).and_then(Entry::from_value)
    }

    /// Every entry that reads as a connection, in the file's order.
    pub fn all(&self) -> Vec<(String, Entry)> {
        self.entries
            .iter()
            .filter_map(|(k, v)| Entry::from_value(v).map(|e| (k.clone(), e)))
            .collect()
    }

    /// Writes one entry; the rest of the file stays as it was.
    pub fn insert(&mut self, name: &str, entry: &Entry) -> Result<()> {
        self.entries.insert(name.to_owned(), entry.to_value());
        self.save()
    }

    /// Removes one entry. `Ok(false)` when there was none.
    pub fn remove(&mut self, name: &str) -> Result<bool> {
        if self.entries.remove(name).is_none() {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Backs the file up beside itself, then replaces it atomically, readable by its owner only.
    fn save(&self) -> Result<()> {
        let dir = self
            .path
            .parent()
            .context("the credentials file has a parent directory")?;
        create_private_dir(dir)?;
        if self.path.exists() {
            let backup = self.path.with_extension("json.bak");
            fs::copy(&self.path, &backup).with_context(|| format!("backing up to {}", backup.display()))?;
            set_private(&backup)?;
        }
        let text = serde_json::to_string_pretty(&Value::Object(self.entries.clone()))? + "\n";
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&tmp)
                .with_context(|| format!("writing {}", tmp.display()))?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &self.path).with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

fn create_private_dir(dir: &Path) -> Result<()> {
    if dir.as_os_str().is_empty() || dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_best_mcp() {
        assert!(valid_name("dotquant.io"));
        assert!(valid_name("admin_dotquant"));
        assert!(valid_name("lab-peer"));
        assert!(!valid_name("-peer"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"x".repeat(65)));
    }

    #[test]
    fn the_device_request_carries_one_lowercase_word() {
        assert_eq!(connection_word("dotquant.io"), "dotquant-io");
        assert_eq!(connection_word("Admin_DotQuant"), "admin-dotquant");
        assert_eq!(connection_word("lab-peer"), "lab-peer");
        assert_eq!(connection_word("..."), "best");
    }

    #[test]
    fn an_entry_round_trips_in_best_mcp_shape() {
        let raw = serde_json::json!({
            "tenantId": "acme", "apiKey": "k", "authType": "apikey", "authHeader": "X-Api-Key",
            "issuedAt": "2026-10-17T10:00:00.000Z", "manifest": "https://api.example.com/.well-known/best",
            "endpoints": {"com.example.payroll": "https://api.example.com/api/best/tenants/acme"}
        });
        let entry = Entry::from_value(&raw).unwrap();
        assert_eq!(entry.host(), "api.example.com");
        assert_eq!(
            entry.endpoint(None).unwrap().1,
            "https://api.example.com/api/best/tenants/acme"
        );
        assert!(entry.endpoint(Some("io.other")).is_err());
        assert_eq!(entry.to_value(), raw);
        assert_eq!(
            format!("{:?}", entry.auth),
            r#"ApiKey { header: "X-Api-Key", key: <credential> }"#
        );
    }

    #[test]
    fn a_bearer_entry_and_a_public_one() {
        let bearer =
            Entry::from_value(&serde_json::json!({"apiKey": "t", "authType": "bearer", "endpoints": {}})).unwrap();
        assert!(matches!(bearer.auth, Auth::Bearer(_)));
        let public = Entry::from_value(&serde_json::json!({"authType": "none", "endpoints": {}})).unwrap();
        assert_eq!(public.auth, Auth::None);
    }
}
