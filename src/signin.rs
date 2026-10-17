//! Sign-in by the device flow (RFC 8628, as SPEC Agent Registration has it): the program asks, the person approves
//! in their own browser, the program polls and keeps the credential. The device code is a secret the service made;
//! it stays in this module.

use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde_json::Value;
use url::Url;

use crate::credentials::Secret;
use crate::http::{parse_error_body, Http};
use crate::manifest::Registration;

pub const CLIENT_ID: &str = "best-cli";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// What the person is shown, and what the poll needs.
#[derive(Debug)]
pub struct Started {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
    /// The `note` of the device answer: the service's guidance for this sign-in, as it wrote it.
    pub note: Option<String>,
    /// A code the person already held: the first poll is made at once.
    held: bool,
    device_code: Secret,
    token_url: Url,
}

/// The token answer, with the credential as a secret.
#[derive(Debug)]
pub struct Issued {
    pub access_token: Secret,
    pub token_type: String,
    pub auth_header: Option<String>,
    pub tenant_id: Option<String>,
    pub expires_in: Option<u64>,
}

/// The person did not approve, or the code ran out: the sign-in is over.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Denied(pub String);

/// RFC 8628 §3.1–3.2: opens the registration.
pub async fn start(http: &Http, registration: &Registration, connection: &str, label: &str) -> Result<Started> {
    let form = [
        ("client_id", CLIENT_ID),
        ("connection", connection),
        ("agent_label", label),
    ];
    let response = http.post_form(&registration.device_authorization_url, &form).await?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let (Some(device_code), Some(user_code), Some(verification_uri)) = (
        json["device_code"].as_str(),
        json["user_code"].as_str(),
        json["verification_uri"].as_str(),
    ) else {
        let (code, message, _) = parse_error_body(&text);
        bail!(
            "{} refused to open a sign-in: {}{message} (HTTP {status})",
            registration.device_authorization_url,
            code.map(|c| format!("{c}: ")).unwrap_or_default()
        );
    };
    Ok(Started {
        user_code: user_code.to_owned(),
        verification_uri: verification_uri.to_owned(),
        verification_uri_complete: json["verification_uri_complete"].as_str().map(str::to_owned),
        expires_in: json["expires_in"].as_u64().filter(|n| *n > 0).unwrap_or(900),
        interval: json["interval"].as_u64().filter(|n| *n > 0).unwrap_or(5),
        note: json["note"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .map(str::to_owned),
        held: false,
        device_code: Secret::new(device_code.to_owned()),
        token_url: registration.token_url.clone(),
    })
}

/// A device code the person already holds (a registration started on another device, or at an authorization URL
/// this machine cannot reach): redeemed at the token endpoint like any other.
pub fn held(token_url: &Url, device_code: String) -> Started {
    Started {
        user_code: String::new(),
        verification_uri: String::new(),
        verification_uri_complete: None,
        expires_in: 900,
        interval: 5,
        note: None,
        held: true,
        device_code: Secret::new(device_code),
        token_url: token_url.clone(),
    }
}

/// One poll of the token endpoint (RFC 8628 §3.4–3.5).
enum Poll {
    Pending,
    SlowDown,
    Issued(Issued),
}

async fn poll(http: &Http, started: &Started) -> Result<Poll> {
    let form = [
        ("grant_type", DEVICE_CODE_GRANT),
        ("device_code", started.device_code.expose()),
        ("client_id", CLIENT_ID),
    ];
    let response = http.post_form(&started.token_url, &form).await?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let (code, message, _) = parse_error_body(&text);
        return match code.as_deref() {
            Some("authorization_pending") => Ok(Poll::Pending),
            Some("slow_down") => Ok(Poll::SlowDown),
            Some(c @ ("access_denied" | "expired_token")) => Err(Denied(format!("{c}: {message}")).into()),
            Some(c) => bail!("{} answered {c}: {message} (HTTP {status})", started.token_url),
            None => bail!("{} answered: {message} (HTTP {status})", started.token_url),
        };
    }
    let Some(access_token) = json["access_token"].as_str() else {
        bail!("{} answered {status} without an access_token", started.token_url);
    };
    Ok(Poll::Issued(Issued {
        access_token: Secret::new(access_token.to_owned()),
        token_type: json["token_type"].as_str().unwrap_or("").to_owned(),
        auth_header: json["auth_header"].as_str().map(str::to_owned),
        tenant_id: json["tenant_id"].as_str().map(str::to_owned),
        expires_in: json["expires_in"].as_u64(),
    }))
}

/// Polls no faster than the service asked until the credential is issued, the person declines, or the code
/// expires. `waiting` is told at each pending poll.
pub async fn wait(http: &Http, started: &Started, waiting: impl Fn()) -> Result<Issued> {
    let deadline = Instant::now() + Duration::from_secs(started.expires_in);
    let mut interval = Duration::from_secs(started.interval);
    let mut first = started.held;
    loop {
        if !std::mem::take(&mut first) {
            tokio::time::sleep(interval).await;
        }
        if Instant::now() > deadline {
            return Err(Denied("the code expired before it was approved".into()).into());
        }
        match poll(http, started).await? {
            Poll::Pending => waiting(),
            Poll::SlowDown => {
                interval += Duration::from_secs(5);
                waiting();
            }
            Poll::Issued(issued) => return Ok(issued),
        }
    }
}
