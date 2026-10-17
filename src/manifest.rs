//! The manifest at `/.well-known/best`: how an address becomes one, what it says about signing in, and which
//! endpoints a credential opens (SPEC Discovery, Multi-Tenancy, Agent Registration).

use anyhow::{bail, Context, Result};
use serde_json::Value;
use url::Url;

use crate::credentials::Auth;
use crate::http::{Error, Http};

const MAX_REDIRECTS: usize = 5;

/// The address a person gave, as a manifest URL: a bare host means its well-known path; `https` anywhere, plain
/// `http` only on this machine's loopback, which the person typing it has opted into.
pub fn address(given: &str) -> Result<Url> {
    let raw = given.trim();
    if raw.is_empty() {
        bail!("no manifest address");
    }
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let mut url = Url::parse(&with_scheme).with_context(|| format!("'{given}' is not an address"))?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/.well-known/best");
    }
    check_scheme(&url, given)?;
    Ok(url)
}

fn check_scheme(url: &Url, given: &str) -> Result<()> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(url) => Ok(()),
        "http" => bail!(
            "'{given}' is plain http: a BEST manifest is read over https, and over http only on this machine (localhost, 127.0.0.1, ::1). Nothing was sent."
        ),
        other => bail!("'{given}' is {other}:, not https. Nothing was sent."),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost" || d.ends_with(".localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// What the manifest says, read as data.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub url: Url,
    pub value: Value,
}

/// The two endpoints a device-flow sign-in needs.
#[derive(Debug, Clone)]
pub struct Registration {
    pub device_authorization_url: Url,
    pub token_url: Url,
    /// The `note` of the block that declares the device flow: the service's sign-in guidance, as it wrote it.
    pub note: Option<String>,
}

impl Manifest {
    fn best(&self) -> &Value {
        &self.value["best"]
    }

    /// The authentication block that governs the surfaces: the root's.
    pub fn authentication(&self) -> &Value {
        &self.best()["authentication"]
    }

    /// Whether the root declares no credential at all.
    pub fn is_public(&self) -> bool {
        let auth = self.authentication();
        auth.is_null() || auth["type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("none"))
    }

    /// The block that declares agent registration: the root's, else the first service's that does.
    pub fn registration(&self) -> Result<Option<Registration>> {
        let mut blocks = vec![self.authentication()];
        if let Some(services) = self.best()["services"].as_object() {
            blocks.extend(services.values().map(|s| &s["authentication"]));
        }
        for block in blocks {
            let (Some(d), Some(t)) = (block["deviceAuthorizationUrl"].as_str(), block["tokenUrl"].as_str()) else {
                continue;
            };
            return Ok(Some(Registration {
                device_authorization_url: Url::parse(d)
                    .with_context(|| format!("deviceAuthorizationUrl '{d}' is not a URL"))?,
                token_url: Url::parse(t).with_context(|| format!("tokenUrl '{t}' is not a URL"))?,
                note: block["note"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .map(str::to_owned),
            }));
        }
        Ok(None)
    }

    /// The token endpoint of the first authentication block that declares one: all a device code held already
    /// needs.
    pub fn token_url(&self) -> Result<Option<Url>> {
        let mut blocks = vec![self.authentication()];
        if let Some(services) = self.best()["services"].as_object() {
            blocks.extend(services.values().map(|s| &s["authentication"]));
        }
        match blocks.into_iter().find_map(|b| b["tokenUrl"].as_str()) {
            Some(t) => Ok(Some(
                Url::parse(t).with_context(|| format!("tokenUrl '{t}' is not a URL"))?,
            )),
            None => Ok(None),
        }
    }

    /// `tenants.manifest`, the RFC 6570 template of a multi-tenant host.
    pub fn tenant_template(&self) -> Option<&str> {
        self.best()["tenants"]["manifest"].as_str()
    }

    /// Service id → `http.endpoint`, in the manifest's order.
    pub fn endpoints(&self) -> Vec<(String, String)> {
        self.best()["services"]
            .as_object()
            .map(|services| {
                services
                    .iter()
                    .filter_map(|(id, s)| {
                        s["http"]["endpoint"]
                            .as_str()
                            .map(|e| (id.clone(), e.trim_end_matches('/').to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn host(&self) -> String {
        host_of(&self.url)
    }
}

fn host_of(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_owned(),
        _ => String::new(),
    }
}

/// Fetches the manifest an address names, following redirects hop by hop (each checked like the first) so the final
/// URL is the canonical one (SPEC Origin Discovery).
pub async fn resolve(http: &Http, given: &str) -> Result<Manifest> {
    let mut url = address(given)?;
    for _ in 0..=MAX_REDIRECTS {
        let response = http.get_public(&url).await?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|l| l.to_str().ok())
                .with_context(|| format!("{url} redirected without saying where"))?;
            let next = url
                .join(location)
                .with_context(|| format!("{url} redirected to '{location}', which is not a URL"))?;
            check_scheme(&next, next.as_str())?;
            url = next;
            continue;
        }
        if !status.is_success() {
            let error = Error::from_response(response).await;
            bail!("{url} is not a BEST manifest: {error}");
        }
        let text = response.text().await.map_err(|e| Error::Unreachable(e.to_string()))?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("{url} is not a BEST manifest: the answer is not JSON"))?;
        if !value["best"].is_object() {
            bail!("{url} is not a BEST manifest: the answer has no \"best\" member. Check the address.");
        }
        return Ok(Manifest { url, value });
    }
    bail!("{given}: more than {MAX_REDIRECTS} redirects")
}

/// Fetches a manifest at a known URL with a credential (a tenant manifest).
async fn fetch_with(http: &Http, url: &Url, auth: &Auth) -> Result<Manifest> {
    let value = http
        .get_json(url, auth)
        .await
        .with_context(|| format!("reading {url}"))?;
    if !value["best"].is_object() {
        bail!("{url} is not a BEST manifest: the answer has no \"best\" member");
    }
    Ok(Manifest {
        url: url.clone(),
        value,
    })
}

/// The endpoints a sign-in opens, as best-mcp resolves them: the tenant manifest's services when the root is
/// multi-tenant and the token answer named a tenant, else the root's; only those on the manifest's own host, where
/// alone the credential may go (SPEC Name Resolution).
/// A tenant id that can be a path segment, as best-mcp accepts one; anything else (a host with a port, as a door
/// answers for a credential at its root) names no tenant manifest.
fn is_tenant_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

pub async fn endpoints_for(
    http: &Http,
    root: &Manifest,
    tenant_id: Option<&str>,
    auth: &Auth,
) -> Result<Vec<(String, String)>> {
    let host = root.host();
    let services = match (root.tenant_template(), tenant_id) {
        (Some(template), Some(tenant)) if is_tenant_id(tenant) => {
            let tenant_url = Url::parse(&template.replace("{tenantId}", &urlencode(tenant)))
                .with_context(|| format!("tenants.manifest '{template}' does not expand to a URL"))?;
            if host_of(&tenant_url) != host {
                bail!(
                    "the tenant manifest is on {}, not on {host} where this sign-in resolved; the credential is not sent there",
                    host_of(&tenant_url)
                );
            }
            fetch_with(http, &tenant_url, auth).await?.endpoints()
        }
        _ => root.endpoints(),
    };
    let mut kept = Vec::new();
    let mut elsewhere = Vec::new();
    for (id, endpoint) in services {
        match Url::parse(&endpoint) {
            Ok(u) if host_of(&u) == host => kept.push((id, endpoint)),
            Ok(u) => elsewhere.push(format!("{id} ({})", host_of(&u))),
            Err(_) => elsewhere.push(format!("{id} ({endpoint})")),
        }
    }
    if kept.is_empty() {
        if elsewhere.is_empty() {
            bail!("the manifest names no service endpoint");
        }
        bail!(
            "every service endpoint the manifest names is on another host than {host} ({}); the credential is not sent there",
            elsewhere.join(", ")
        );
    }
    Ok(kept)
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bare_host_means_its_well_known_path() {
        assert_eq!(
            address("api.example.com").unwrap().as_str(),
            "https://api.example.com/.well-known/best"
        );
        assert_eq!(
            address("https://api.example.com/").unwrap().as_str(),
            "https://api.example.com/.well-known/best"
        );
        assert_eq!(
            address("https://baas.dotquant.io/.well-known/best/admin")
                .unwrap()
                .as_str(),
            "https://baas.dotquant.io/.well-known/best/admin"
        );
        assert_eq!(
            address("http://127.0.0.1:7777").unwrap().as_str(),
            "http://127.0.0.1:7777/.well-known/best"
        );
        assert_eq!(
            address("localhost:7175").unwrap().as_str(),
            "https://localhost:7175/.well-known/best"
        );
    }

    #[test]
    fn a_tenant_id_is_a_path_segment() {
        assert!(is_tenant_id("acme"));
        assert!(is_tenant_id("be9e0176-d898-40a0-b558-eaa6f748b6aa"));
        assert!(!is_tenant_id("127.0.0.1:7791"), "a door's host is not a tenant");
        assert!(!is_tenant_id(""));
    }

    #[test]
    fn plain_http_is_for_this_machine_only() {
        assert!(address("http://api.example.com").is_err());
        assert!(address("ftp://x").is_err());
        assert!(address("http://localhost:7777").is_ok());
        assert!(address("http://[::1]:7777").is_ok());
    }

    fn manifest(value: Value) -> Manifest {
        Manifest {
            url: Url::parse("https://api.example.com/.well-known/best").unwrap(),
            value,
        }
    }

    #[test]
    fn registration_is_the_roots_else_a_services() {
        let m = manifest(json!({"best": {"authentication": {"type": "apiKey"}, "services": {
            "com.example.pub": {"authentication": {"type": "none"}, "http": {"endpoint": "https://api.example.com/pub"}},
            "com.example.x": {"authentication": {"deviceAuthorizationUrl": "https://api.example.com/auth/device", "tokenUrl": "https://api.example.com/auth/token"}, "http": {"endpoint": "https://api.example.com/x"}}
        }}}));
        let r = m.registration().unwrap().unwrap();
        assert_eq!(r.token_url.as_str(), "https://api.example.com/auth/token");
        assert_eq!(
            m.endpoints(),
            vec![
                ("com.example.pub".into(), "https://api.example.com/pub".into()),
                ("com.example.x".into(), "https://api.example.com/x".into())
            ]
        );
        assert!(!m.is_public());
        assert!(manifest(json!({"best": {"services": {}}})).is_public());
        assert!(manifest(json!({"best": {"authentication": {"type": "apiKey"}}}))
            .registration()
            .unwrap()
            .is_none());
    }
}
