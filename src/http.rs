//! HTTP against a BEST surface: the credential goes in the header the connection names and follows no redirect,
//! a refusal is the service's own words, and an error says enough for the exit code to be meaningful.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use url::Url;

use crate::credentials::Auth;

pub const USER_AGENT: &str = concat!("best-cli/", env!("CARGO_PKG_VERSION"));
const CLOUDEVENTS_JSON: &str = "application/cloudevents+json";

/// What a request can fail with. The variants map to exit codes in `main`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The service refused: a BEST error body, or an RFC 6749 one, or whatever it said.
    #[error("{}", describe(.status, .code.as_deref(), .message, .details.as_ref()))]
    Refused {
        status: u16,
        code: Option<String>,
        message: String,
        details: Option<Value>,
    },
    /// 401: no credential, or one the service no longer accepts.
    #[error("{}", describe(.status, .code.as_deref(), .message, .details.as_ref()))]
    Unauthorized {
        status: u16,
        code: Option<String>,
        message: String,
        details: Option<Value>,
    },
    /// 5xx: the service failed.
    #[error("{}", describe(.status, .code.as_deref(), .message, .details.as_ref()))]
    Failed {
        status: u16,
        code: Option<String>,
        message: String,
        details: Option<Value>,
    },
    /// The service could not be reached, or the connection broke.
    #[error("{0}")]
    Unreachable(String),
    /// A 2xx whose body was not what the protocol says.
    #[error("{0}")]
    Unexpected(String),
}

impl Error {
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Refused { code, .. } | Error::Unauthorized { code, .. } | Error::Failed { code, .. } => {
                code.as_deref()
            }
            _ => None,
        }
    }

    /// The service's message alone, for `--json`.
    pub fn message(&self) -> String {
        match self {
            Error::Refused { message, .. } | Error::Unauthorized { message, .. } | Error::Failed { message, .. } => {
                message.clone()
            }
            Error::Unreachable(m) | Error::Unexpected(m) => m.clone(),
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Refused { status, .. } | Error::Unauthorized { status, .. } | Error::Failed { status, .. } => {
                Some(*status)
            }
            _ => None,
        }
    }

    pub fn details(&self) -> Option<&Value> {
        match self {
            Error::Refused { details, .. } | Error::Unauthorized { details, .. } | Error::Failed { details, .. } => {
                details.as_ref()
            }
            _ => None,
        }
    }

    /// The service's answer, classified by status.
    pub async fn from_response(response: reqwest::Response) -> Self {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        let (code, message, details) = parse_error_body(&text);
        match status {
            401 => Error::Unauthorized {
                status,
                code,
                message,
                details,
            },
            500..=599 => Error::Failed {
                status,
                code,
                message,
                details,
            },
            _ => Error::Refused {
                status,
                code,
                message,
                details,
            },
        }
    }
}

fn describe(status: &u16, code: Option<&str>, message: &str, details: Option<&Value>) -> String {
    let mut text = match code {
        Some(c) => format!("{c}: {message}"),
        None => message.to_owned(),
    };
    if let Some(d) = details {
        text.push_str(&format!("\ndetails: {d}"));
    }
    format!("{text} (HTTP {status})")
}

/// A BEST error (`{error:{code,message,details}}`), an RFC 6749 error (`{error, error_description}`), or the text.
pub fn parse_error_body(text: &str) -> (Option<String>, String, Option<Value>) {
    if let Ok(json) = serde_json::from_str::<Value>(text) {
        match json.get("error") {
            Some(Value::Object(err)) => {
                let code = err.get("code").and_then(Value::as_str).map(str::to_owned);
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| Value::Object(err.clone()).to_string());
                return (code, message, err.get("details").cloned());
            }
            Some(Value::String(code)) => {
                let message = json
                    .get("error_description")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| code.clone());
                return (Some(code.clone()), message, None);
            }
            _ => {}
        }
        for key in ["message", "detail", "title"] {
            if let Some(m) = json.get(key).and_then(Value::as_str) {
                return (None, m.to_owned(), None);
            }
        }
    }
    let text = text.trim();
    let message = if text.is_empty() {
        "the service answered with an empty body".to_owned()
    } else {
        text.chars().take(500).collect()
    };
    (None, message, None)
}

/// The client every request goes through: no redirects (a credential never follows one), bounded waits.
#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    pub fn new(ca_file: Option<&Path>) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(120));
        if let Some(path) = ca_file {
            let pem = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let certificates = reqwest::Certificate::from_pem_bundle(&pem)
                .with_context(|| format!("{} is not PEM", path.display()))?;
            if certificates.is_empty() {
                anyhow::bail!("{} holds no certificate", path.display());
            }
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        Ok(Self {
            client: builder.build().context("building the HTTP client")?,
        })
    }

    fn authed(&self, request: reqwest::RequestBuilder, auth: &Auth) -> reqwest::RequestBuilder {
        match auth {
            Auth::None => request,
            Auth::Bearer(key) => request.header("Authorization", format!("Bearer {}", key.expose())),
            Auth::ApiKey { header, key } => request.header(header.as_str(), key.expose()),
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, Error> {
        request
            .send()
            .await
            .map_err(|e| Error::Unreachable(unreachable_text(&e)))
    }

    /// `GET`, expecting JSON.
    pub async fn get_json(&self, url: &Url, auth: &Auth) -> Result<Value, Error> {
        let response = self
            .send(
                self.authed(self.client.get(url.clone()), auth)
                    .header("Accept", "application/json"),
            )
            .await?;
        self.json_of(response).await
    }

    /// `GET` with no credential, redirects followed by the caller (the manifest).
    pub async fn get_public(&self, url: &Url) -> Result<reqwest::Response, Error> {
        self.send(self.client.get(url.clone()).header("Accept", "application/json"))
            .await
    }

    /// A form `POST` with no credential: the device and token endpoints.
    pub async fn post_form(&self, url: &Url, form: &[(&str, &str)]) -> Result<reqwest::Response, Error> {
        self.send(
            self.client
                .post(url.clone())
                .header("Accept", "application/json")
                .form(form),
        )
        .await
    }

    /// `POST /commands` as CloudEvents structured content; a service that does not take that content type
    /// (415) gets plain JSON.
    pub async fn post_command(&self, url: &Url, auth: &Auth, envelope: &Value) -> Result<Value, Error> {
        let body = serde_json::to_vec(envelope).map_err(|e| Error::Unexpected(e.to_string()))?;
        let post = |content_type: &'static str| {
            self.authed(self.client.post(url.clone()), auth)
                .header("Accept", "application/json")
                .header("Content-Type", content_type)
                .body(body.clone())
        };
        let mut response = self.send(post(CLOUDEVENTS_JSON)).await?;
        if response.status().as_u16() == 415 {
            response = self.send(post("application/json")).await?;
        }
        self.json_of(response).await
    }

    /// Opens the live stream; the caller reads it frame by frame.
    pub async fn open_stream(
        &self,
        url: &Url,
        auth: &Auth,
        last_event_id: Option<&str>,
    ) -> Result<reqwest::Response, Error> {
        let mut request = self
            .authed(self.client.get(url.clone()), auth)
            .header("Accept", "text/event-stream");
        if let Some(id) = last_event_id {
            request = request.header("Last-Event-ID", id);
        }
        let response = self.send(request).await?;
        if !response.status().is_success() {
            return Err(Error::from_response(response).await);
        }
        Ok(response)
    }

    async fn json_of(&self, response: reqwest::Response) -> Result<Value, Error> {
        if !response.status().is_success() {
            return Err(Error::from_response(response).await);
        }
        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| Error::Unreachable(e.to_string()))?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| {
            Error::Unexpected(format!(
                "the service answered {status} with something that is not JSON: {}",
                text.chars().take(300).collect::<String>()
            ))
        })
    }
}

fn unreachable_text(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        let s_text = s.to_string();
        if !text.contains(&s_text) {
            text.push_str(": ");
            text.push_str(&s_text);
        }
        source = s.source();
    }
    text
}

/// `endpoint` + `path`: the leading slash is a separator (SPEC HTTP Transport).
pub fn join(endpoint: &str, path: &str) -> Result<Url> {
    let joined = format!("{}/{}", endpoint.trim_end_matches('/'), path.trim_start_matches('/'));
    Url::parse(&joined).with_context(|| format!("'{joined}' is not a URL"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_errors_keep_code_message_and_details() {
        let (code, message, details) = parse_error_body(
            r#"{"error":{"code":"SCHEMA_VALIDATION_FAILED","message":"data.model is required","details":{"field":"model"}}}"#,
        );
        assert_eq!(code.as_deref(), Some("SCHEMA_VALIDATION_FAILED"));
        assert_eq!(message, "data.model is required");
        assert_eq!(details.unwrap()["field"], "model");
    }

    #[test]
    fn oauth_errors_and_plain_text_are_kept_too() {
        let (code, message, _) =
            parse_error_body(r#"{"error":"access_denied","error_description":"the person said no"}"#);
        assert_eq!(code.as_deref(), Some("access_denied"));
        assert_eq!(message, "the person said no");
        let (code, message, _) = parse_error_body("gateway timeout");
        assert!(code.is_none());
        assert_eq!(message, "gateway timeout");
        let (_, message, _) = parse_error_body("");
        assert!(message.contains("empty body"));
    }

    #[test]
    fn paths_append_to_the_endpoint() {
        assert_eq!(
            join("https://api.example.com/api/best/tenants/acme/", "/commands")
                .unwrap()
                .as_str(),
            "https://api.example.com/api/best/tenants/acme/commands"
        );
        assert_eq!(
            join("https://app.example.com", "queries/x?a=1").unwrap().as_str(),
            "https://app.example.com/queries/x?a=1"
        );
    }
}
