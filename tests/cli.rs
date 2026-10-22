//! The binary, end to end, against a fake of the BEST HTTP surface: a multi-tenant host with the device flow, the
//! catalogues, a query, the command ingest, the event log and its stream. The credentials file is a temporary
//! copy of best-mcp's shape, seeded with an entry this program did not write, which must come out as it went in.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TENANT: &str = "acme";
const KEY: &str = "sk-the-minted-key-never-shown";
const NAME: &str = "acme-test";

/// An entry best-mcp wrote, with a member this program does not know.
fn foreign_entry(base: &str) -> Value {
    json!({
        "tenantId": "other",
        "apiKey": "foreign-key",
        "authType": "apikey",
        "authHeader": "X-Api-Key",
        "issuedAt": "2026-09-24T12:41:58.034Z",
        "supersededKeyHash": "abc",
        "manifest": format!("{base}/.well-known/best"),
        "endpoints": {"com.example.payroll": format!("{base}/api/best/tenants/other")}
    })
}

struct Rig {
    server: MockServer,
    home: TempDir,
}

impl Rig {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let base = server.uri();
        let home = TempDir::new().unwrap();
        std::fs::write(
            home.path().join("credentials.json"),
            serde_json::to_string_pretty(&json!({"dotquant.io": foreign_entry(&base)})).unwrap() + "\n",
        )
        .unwrap();

        Mock::given(method("GET"))
            .and(path("/.well-known/best"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
                "version": "0.9.17",
                "authentication": {
                    "type": "apiKey", "in": "header", "scheme": "X-Api-Key",
                    "tokenUrl": format!("{base}/auth/token"),
                    "deviceAuthorizationUrl": format!("{base}/auth/device"),
                    "note": "manifest text"
                },
                "tenants": {"manifest": format!("{base}/.well-known/best/{{tenantId}}")},
                "services": {"com.example.payroll": {"version": "1.0.0", "description": "Payroll", "http": {"endpoint": format!("{base}/api/best/tenants")}}},
                "capabilities": []
            }})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/.well-known/best/{TENANT}")))
            .and(header("X-Api-Key", KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
                "version": "0.9.17",
                "services": {
                    "com.example.payroll": {"version": "1.0.0", "description": "Payroll", "http": {"endpoint": format!("{base}/api/best/tenants/{TENANT}")}},
                    "com.example.elsewhere": {"version": "1.0.0", "description": "Not here", "http": {"endpoint": "https://other.example.com/x"}}
                },
                "capabilities": []
            }})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/device"))
            .and(body_string_contains("client_id=best-cli"))
            .and(body_string_contains(format!("connection={NAME}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "dc-secret", "user_code": "WXYZ-1234",
                "verification_uri": format!("{base}/activate"),
                "verification_uri_complete": format!("{base}/activate?code=WXYZ-1234"),
                "expires_in": 600, "interval": 1, "note": "device text"
            })))
            .mount(&server)
            .await;
        // The first poll finds the person has not acted yet; the second has the credential.
        Mock::given(method("POST"))
            .and(path("/auth/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "authorization_pending"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/token"))
            .and(body_string_contains(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
            ))
            .and(body_string_contains("device_code=dc-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": KEY, "token_type": "apikey", "auth_header": "X-Api-Key",
                "tenant_id": TENANT, "name": NAME,
                "endpoint": format!("{base}/api/best/tenants/{TENANT}"),
                "note": "The credential is a secret."
            })))
            .mount(&server)
            .await;
        Self { server, home }
    }

    fn tenant(&self, rest: &str) -> String {
        format!("/api/best/tenants/{TENANT}/{rest}")
    }

    fn credentials_file(&self) -> PathBuf {
        self.home.path().join("credentials.json")
    }

    fn credentials(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.credentials_file()).unwrap()).unwrap()
    }

    async fn run(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        self.run_with_stdin(args, "").await
    }

    async fn run_with_stdin(&self, args: &[&str], stdin: &str) -> assert_cmd::assert::Assert {
        let file = self.credentials_file();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let stdin = stdin.to_owned();
        tokio::task::spawn_blocking(move || {
            Command::cargo_bin("best")
                .unwrap()
                .env("BEST_MCP_CREDENTIALS_FILE", &file)
                .env_remove("BEST_CA_FILE")
                .args(&args)
                .write_stdin(stdin)
                .assert()
        })
        .await
        .unwrap()
    }

    async fn login(&self) {
        let out = self
            .run(&["login", NAME, "--manifest", &self.server.uri()])
            .await
            .success()
            .stderr(predicate::str::contains("/activate?code=WXYZ-1234"))
            .stderr(predicate::str::contains("The code is WXYZ-1234"))
            .stderr(predicate::str::contains(
                "The service's sign-in guidance, as it wrote it:\n\nmanifest text\n\ndevice text\n",
            ))
            .stderr(predicate::str::contains(format!(
                "Signed in as '{NAME}' (tenant {TENANT})"
            )));
        assert_never_shows_key(&out);
    }

    /// Seeds a signed-in entry directly, as best-mcp would have left it, so tests need no device flow.
    fn signed_in(&self) {
        let base = self.server.uri();
        let mut all = self.credentials();
        all[NAME] = json!({
            "tenantId": TENANT, "apiKey": KEY, "authType": "apikey", "authHeader": "X-Api-Key",
            "issuedAt": "2026-10-17T10:00:00.000Z", "manifest": format!("{base}/.well-known/best"),
            "endpoints": {"com.example.payroll": format!("{base}/api/best/tenants/{TENANT}")}
        });
        std::fs::write(self.credentials_file(), serde_json::to_string_pretty(&all).unwrap()).unwrap();
    }

    async fn catalogues(&self) {
        let base = self.server.uri();
        Mock::given(method("GET"))
            .and(path(self.tenant("commands")))
            .and(header("X-Api-Key", KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"commands": [
                {"schema": "submit-order", "version": "1.0", "commandType": "SubmitOrder",
                 "dataschema": format!("{base}/api/best/tenants/{TENANT}/commands/submit-order/1.0"),
                 "description": "Submit a market order.\nMore detail.",
                 "impact": {"categories": ["financial"], "confirmation": "required", "warning": "Places a real order. Capital is at risk."}},
                {"schema": "follow-symbol", "version": "1.0",
                 "dataschema": format!("{base}/api/best/tenants/{TENANT}/commands/follow-symbol/1.0"),
                 "description": "Follow a symbol — harmless"},
                {"schema": "follow-symbol", "version": "2.0", "commandType": "FollowSymbol",
                 "dataschema": format!("{base}/api/best/tenants/{TENANT}/commands/follow-symbol/2.0"),
                 "description": "Follow a symbol, second version"}
            ]})))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path(self.tenant("queries")))
            .and(header("X-Api-Key", KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"queries": [
                {"schema": "list-things", "version": "1.0", "dataschema": format!("{base}/api/best/tenants/{TENANT}/queries/list-things/1.0"), "description": "The things"},
                {"schema": "get-total", "version": "1.0", "dataschema": format!("{base}/api/best/tenants/{TENANT}/queries/get-total/1.0"), "description": "One number"}
            ]})))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path(self.tenant("commands/submit-order/1.0")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "object", "commandType": "SubmitOrder", "properties": {"Symbol": {"type": "string"}}, "required": ["Symbol"]
            })))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path(self.tenant("queries/list-things/1.0")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "parameters": {"type": "object", "properties": {"status": {"type": "string"}}}, "response": {"type": "object"}
            })))
            .mount(&self.server)
            .await;
    }

    async fn requests(&self, method_name: &str, p: &str) -> Vec<Request> {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == method_name && r.url.path() == p)
            .collect()
    }
}

fn assert_never_shows_key(out: &assert_cmd::assert::Assert) {
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        !stdout.contains(KEY) && !stderr.contains(KEY),
        "the credential was printed:\n{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("dc-secret") && !stderr.contains("dc-secret"),
        "the device code was printed"
    );
}

fn file_mode_is_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let _ = path;
}

#[tokio::test(flavor = "multi_thread")]
async fn login_runs_the_device_flow_and_keeps_the_credential_where_best_mcp_does() {
    let rig = Rig::new().await;
    let base = rig.server.uri();
    rig.login().await;

    let all = rig.credentials();
    let entry = &all[NAME];
    assert_eq!(entry["tenantId"], TENANT);
    assert_eq!(entry["apiKey"], KEY, "the credential is stored, in best-mcp's member");
    assert_eq!(entry["authType"], "apikey");
    assert_eq!(entry["authHeader"], "X-Api-Key");
    assert_eq!(entry["manifest"], format!("{base}/.well-known/best"));
    assert_eq!(
        entry["endpoints"],
        json!({"com.example.payroll": format!("{base}/api/best/tenants/{TENANT}")}),
        "only the tenant manifest's endpoint on the sign-in host; the one elsewhere is not a place for the credential"
    );
    assert!(entry["issuedAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        all["dotquant.io"],
        foreign_entry(&base),
        "the entry best-mcp wrote is untouched, unknown member included"
    );
    assert_eq!(
        all.as_object().unwrap().keys().next().unwrap(),
        "dotquant.io",
        "the file keeps its order"
    );
    file_mode_is_private(&rig.credentials_file());
    assert!(
        rig.home.path().join("credentials.json.bak").exists(),
        "the file was backed up beside itself before the write"
    );

    // The person's poll came no faster than the interval: one pending answer, then the credential.
    assert_eq!(rig.requests("POST", "/auth/token").await.len(), 2);

    rig.run(&["connections"])
        .await
        .success()
        .stdout(predicate::str::contains("dotquant.io"))
        .stdout(predicate::str::contains(NAME))
        .stdout(predicate::str::contains("com.example.payroll"))
        .stdout(predicate::str::contains(KEY).not());
    let out = rig.run(&["connections", "--json"]).await.success();
    let listed: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(listed[1]["name"], NAME);
    assert_eq!(listed[1]["tenantId"], TENANT);
    assert_never_shows_key(&out);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_name_is_never_signed_in_again() {
    let rig = Rig::new().await;
    let before = std::fs::read_to_string(rig.credentials_file()).unwrap();
    rig.run(&["login", "dotquant.io", "--manifest", &rig.server.uri()])
        .await
        .code(1)
        .stderr(predicate::str::contains("already signed in"))
        .stderr(predicate::str::contains("best logout dotquant.io"));
    assert_eq!(
        std::fs::read_to_string(rig.credentials_file()).unwrap(),
        before,
        "nothing was written"
    );
    assert!(
        rig.requests("POST", "/auth/device").await.is_empty(),
        "nothing was sent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_name_and_a_bad_address_are_usage_errors_before_anything_is_sent() {
    let rig = Rig::new().await;
    rig.run(&["login", "-bad", "--manifest", &rig.server.uri()])
        .await
        .code(2);
    rig.run(&["login", "fine", "--manifest", "http://api.example.com"])
        .await
        .code(1)
        .stderr(predicate::str::contains("plain http"));
    assert!(rig.requests("GET", "/.well-known/best").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_connection_the_person_is_told_how_to_sign_in_and_exit_is_3() {
    let rig = Rig::new().await;
    rig.run(&["queries", "nothere"])
        .await
        .code(3)
        .stderr(predicate::str::contains("best login nothere --manifest"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_catalogues_are_listed_with_descriptions_and_impact() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;

    rig.run(&["commands", NAME])
        .await
        .success()
        .stdout(predicate::str::contains("submit-order"))
        .stdout(predicate::str::contains("financial (required)"))
        .stdout(predicate::str::contains("Submit a market order."))
        .stdout(predicate::str::contains("More detail").not());
    rig.run(&["commands", NAME, "order"])
        .await
        .success()
        .stdout(predicate::str::contains("submit-order"))
        .stdout(predicate::str::contains("follow-symbol").not());
    let out = rig.run(&["commands", NAME, "--json"]).await.success();
    let raw: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(
        raw[0]["impact"]["warning"], "Places a real order. Capital is at risk.",
        "the catalogue as the service sent it"
    );
    rig.run(&["queries", NAME])
        .await
        .success()
        .stdout(predicate::str::contains("list-things"))
        .stdout(predicate::str::contains("One number"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_schema_is_found_among_commands_then_queries() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    let out = rig.run(&["schema", NAME, "SubmitOrder"]).await.success();
    let doc: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(doc["required"][0], "Symbol");
    let out = rig.run(&["schema", NAME, "list-things"]).await.success();
    let doc: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(doc["parameters"].is_object());
    rig.run(&["schema", NAME, "nothing"])
        .await
        .code(1)
        .stderr(predicate::str::contains("no command or query called 'nothing'"));
    rig.run(&["schema", NAME, "thing-happened", "--kind", "event"])
        .await
        .code(2)
        .stderr(predicate::str::contains("needs its version"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_prints_a_table_or_the_json() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .and(query_param("status", "Active"))
        .and(header("X-Api-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "things": [{"id": "t-1", "name": "One", "tags": ["a"]}, {"id": "t-2", "name": "Two", "extra": true}],
            "nextCursor": "c2"
        })))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/get-total")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total": 42})))
        .mount(&rig.server)
        .await;

    rig.run(&["query", NAME, "list-things", "--param", "status=Active"])
        .await
        .success()
        .stdout(predicate::str::contains("id   name  tags   extra"))
        .stdout(predicate::str::contains("t-1  One   [\"a\"]"))
        .stdout(predicate::str::contains("t-2  Two          true"))
        .stdout(predicate::str::contains("nextCursor: c2"));
    let out = rig
        .run(&["query", NAME, "list-things", "-p", "status=Active", "--json"])
        .await
        .success();
    let answer: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(answer["things"][1]["id"], "t-2");
    let out = rig.run(&["query", NAME, "get-total"]).await.success();
    let answer: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(answer["total"], 42, "a scalar answer is JSON");
    rig.run(&["query", NAME, "list-things", "--param", "status"])
        .await
        .code(2);
    rig.run(&["query", NAME, "list-thing"])
        .await
        .code(1)
        .stderr(predicate::str::contains("Did you mean: list-things"));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_builds_the_envelope_as_best_mcp_does_and_honours_impact() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    let base = rig.server.uri();
    Mock::given(method("POST"))
        .and(path(rig.tenant("commands")))
        .and(header("X-Api-Key", KEY))
        .and(header("Content-Type", "application/cloudevents+json"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "e1", "correlationId": "c1"})))
        .mount(&rig.server)
        .await;

    // A high-impact command with no terminal and no --yes is not sent.
    rig.run(&["send", NAME, "submit-order", "--data", r#"{"Symbol": "MSFT"}"#])
        .await
        .code(4)
        .stderr(predicate::str::contains("Capital is at risk"))
        .stderr(predicate::str::contains("--yes"));
    assert!(rig.requests("POST", &rig.tenant("commands")).await.is_empty());

    rig.run(&[
        "send",
        NAME,
        "submit-order",
        "--data",
        r#"{"Symbol": "MSFT"}"#,
        "--yes",
        "--correlation-id",
        "c1",
    ])
    .await
    .success()
    .stderr(predicate::str::contains("Accepted SubmitOrder: id e1, correlation c1"))
    .stderr(predicate::str::contains(format!(
        "best events {NAME} --correlation-id c1 --follow"
    )));
    let posted = rig.requests("POST", &rig.tenant("commands")).await;
    assert_eq!(posted.len(), 1);
    let body: Value = serde_json::from_slice(&posted[0].body).unwrap();
    assert_eq!(body["specversion"], "1.0");
    assert_eq!(body["type"], "SubmitOrder");
    assert_eq!(body["source"], "urn:best-cli");
    assert_eq!(body["datacontenttype"], "application/json");
    assert_eq!(
        body["dataschema"],
        format!("{base}/api/best/tenants/{TENANT}/commands/submit-order/1.0")
    );
    assert_eq!(body["correlationid"], "c1");
    assert_eq!(body["data"], json!({"Symbol": "MSFT"}));
    assert!(uuid::Uuid::parse_str(body["id"].as_str().unwrap()).is_ok());
    assert!(body["time"].as_str().unwrap().ends_with('Z'));

    // A command without the annotation is sent without a question; the type is derived when the catalogue
    // states none, and the latest version is taken.
    let out = rig
        .run(&[
            "send",
            NAME,
            "follow-symbol",
            "--data",
            r#"{"Symbol": "AAPL"}"#,
            "--json",
        ])
        .await
        .success();
    let accepted: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(accepted, json!({"id": "e1", "correlationId": "c1"}));
    let posted = rig.requests("POST", &rig.tenant("commands")).await;
    let body: Value = serde_json::from_slice(&posted[1].body).unwrap();
    assert_eq!(body["type"], "FollowSymbol");
    assert_eq!(
        body["dataschema"],
        format!("{base}/api/best/tenants/{TENANT}/commands/follow-symbol/2.0")
    );
    assert!(body.get("correlationid").is_none());

    // The payload from a file, and a chosen version.
    let file = rig.home.path().join("payload.json");
    std::fs::write(&file, r#"{"Symbol": "NVDA"}"#).unwrap();
    rig.run(&["send", NAME, "follow-symbol/1.0", "--file", file.to_str().unwrap()])
        .await
        .success();
    let posted = rig.requests("POST", &rig.tenant("commands")).await;
    let body: Value = serde_json::from_slice(&posted[2].body).unwrap();
    assert_eq!(
        body["dataschema"],
        format!("{base}/api/best/tenants/{TENANT}/commands/follow-symbol/1.0")
    );
    assert_eq!(body["data"]["Symbol"], "NVDA");

    // From stdin, and a payload that is not an object.
    rig.run_with_stdin(&["send", NAME, "follow-symbol", "--data", "-"], r#"{"Symbol": "X"}"#)
        .await
        .success();
    rig.run(&["send", NAME, "follow-symbol", "--data", "[1]"]).await.code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_shows_the_envelope_and_sends_nothing() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    let out = rig
        .run(&[
            "send",
            NAME,
            "submit-order",
            "--data",
            r#"{"Symbol": "MSFT"}"#,
            "--dry-run",
        ])
        .await
        .success();
    let shown: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(shown["post"], format!("{}{}", rig.server.uri(), rig.tenant("commands")));
    assert_eq!(shown["body"]["type"], "SubmitOrder");
    assert!(rig.requests("POST", &rig.tenant("commands")).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_does_not_take_cloudevents_content_gets_plain_json() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    Mock::given(method("POST"))
        .and(path(rig.tenant("commands")))
        .and(header("Content-Type", "application/cloudevents+json"))
        .respond_with(ResponseTemplate::new(415))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path(rig.tenant("commands")))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "e9"})))
        .mount(&rig.server)
        .await;
    rig.run(&["send", NAME, "follow-symbol"])
        .await
        .success()
        .stderr(predicate::str::contains("id e9"));
    assert_eq!(rig.requests("POST", &rig.tenant("commands")).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_is_the_services_own_words_with_exit_5() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    Mock::given(method("POST"))
        .and(path(rig.tenant("commands")))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "code": "SCHEMA_VALIDATION_FAILED",
            "message": "data.Symbol is required",
            "details": {"field": "Symbol", "fix": "send the ticker"}
        }})))
        .mount(&rig.server)
        .await;

    rig.run(&["send", NAME, "follow-symbol"])
        .await
        .code(5)
        .stderr(predicate::str::contains(
            "SCHEMA_VALIDATION_FAILED: data.Symbol is required",
        ))
        .stderr(predicate::str::contains("send the ticker"))
        .stderr(predicate::str::contains("HTTP 400"));
    let out = rig.run(&["send", NAME, "follow-symbol", "--json"]).await.code(5);
    let err: Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert_eq!(err["error"]["code"], "SCHEMA_VALIDATION_FAILED");
    assert_eq!(err["error"]["message"], "data.Symbol is required");
    assert_eq!(err["error"]["details"]["field"], "Symbol");
    assert_eq!(err["error"]["status"], 400);
    assert_eq!(err["exit"], 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_credential_exits_3_and_a_failing_service_6() {
    let rig = Rig::new().await;
    rig.signed_in();
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries")))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"error": {"code": "UNAUTHORIZED", "message": "that key is not a key of this tenant"}}),
        ))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("commands")))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&rig.server)
        .await;
    rig.run(&["queries", NAME])
        .await
        .code(3)
        .stderr(predicate::str::contains("not a key of this tenant"));
    rig.run(&["commands", NAME])
        .await
        .code(6)
        .stderr(predicate::str::contains("down (HTTP 503)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn events_are_read_as_a_table_and_followed_as_lines() {
    let rig = Rig::new().await;
    rig.signed_in();
    Mock::given(method("GET"))
        .and(path(rig.tenant("events")))
        .and(query_param("correlationId", "c1"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"events": [
            {"specversion": "1.0", "id": "ev1", "type": "OrderSubmitted", "source": "https://x", "correlationid": "c1", "time": "2026-10-17T10:00:00Z", "data": {"Symbol": "MSFT"}},
            {"specversion": "1.0", "id": "ev2", "type": "OrderFilled", "source": "https://x", "correlationid": "c1", "time": "2026-10-17T10:00:01Z", "data": {"qty": 1}}
        ], "nextCursor": "n1"})))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .and(query_param("type", "OrderFilled"))
        .and(header("Accept", "text/event-stream"))
        .and(header("Last-Event-ID", "ev1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(": keepalive\n\nid: ev2\ndata: {\"id\":\"ev2\",\"type\":\"OrderFilled\",\"correlationid\":\"c1\",\"time\":\"t\",\"data\":{\"qty\":1}}\n\nid: ev3\ndata: {\"id\":\"ev3\",\"type\":\"OrderFilled\"}\n\n"),
        )
        .mount(&rig.server)
        .await;

    rig.run(&["events", NAME, "--correlation-id", "c1", "--limit", "2"])
        .await
        .success()
        .stdout(predicate::str::contains("TIME"))
        .stdout(predicate::str::contains("OrderSubmitted"))
        .stdout(predicate::str::contains("{\"qty\":1}"))
        .stderr(predicate::str::contains("More: --after n1"));
    let out = rig
        .run(&[
            "events",
            NAME,
            "--follow",
            "--type",
            "OrderFilled",
            "--last-event-id",
            "ev1",
            "--limit",
            "2",
        ])
        .await
        .success();
    let lines: Vec<Value> = String::from_utf8_lossy(&out.get_output().stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["id"], "ev2");
    assert_eq!(lines[1]["id"], "ev3");

    // Without --last-event-id, what happened comes first (the history, ev1 and ev2), then the stream after the last
    // of it: ev2 is replayed by the service and not shown twice, ev3 is new.
    Mock::given(method("GET"))
        .and(path(rig.tenant("events")))
        .and(query_param("type", "OrderFilled"))
        .and(query_param("limit", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"events": [
            {"id": "ev1", "type": "OrderFilled", "correlationid": "c1", "time": "t1", "data": {}},
            {"id": "ev2", "type": "OrderFilled", "correlationid": "c1", "time": "t2", "data": {"qty": 1}}
        ]})))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .and(query_param("type", "OrderFilled"))
        .and(header("Last-Event-ID", "ev2"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string("id: ev2\ndata: {\"id\":\"ev2\",\"type\":\"OrderFilled\"}\n\nid: ev3\ndata: {\"id\":\"ev3\",\"type\":\"OrderFilled\"}\n\n"),
        )
        .mount(&rig.server)
        .await;
    let out = rig
        .run(&["events", NAME, "--follow", "--type", "OrderFilled", "--limit", "3"])
        .await
        .success();
    let ids: Vec<String> = String::from_utf8_lossy(&out.get_output().stdout)
        .lines()
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(ids, ["ev1", "ev2", "ev3"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn workflows_are_the_index_then_one_recipe() {
    let rig = Rig::new().await;
    rig.signed_in();
    let base = rig.server.uri();
    Mock::given(method("GET"))
        .and(path(rig.tenant("workflows")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"workflows": [{"id": "onboard", "name": "Onboard", "description": "Steps"}]})),
        )
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("workflows/onboard")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "onboard", "name": "Onboard", "description": "Steps", "steps": [
            {"kind": "command", "dataschema": format!("{base}/api/best/tenants/{TENANT}/commands/submit-order/1.0"), "guidance": "Keep the id"},
            {"kind": "query", "dataschema": format!("{base}/api/best/tenants/{TENANT}/queries/list-things/1.0"), "optional": true}
        ]})))
        .mount(&rig.server)
        .await;
    rig.run(&["workflows", NAME])
        .await
        .success()
        .stdout(predicate::str::contains("onboard  Onboard  Steps"));
    rig.run(&["workflows", NAME, "onboard"])
        .await
        .success()
        .stdout(predicate::str::contains("1. command submit-order/1.0"))
        .stdout(predicate::str::contains("   Keep the id"))
        .stdout(predicate::str::contains("2. query list-things/1.0 (optional)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_manifest_is_printed_for_a_connection_or_an_address() {
    let rig = Rig::new().await;
    rig.signed_in();
    let out = rig.run(&["manifest", NAME]).await.success();
    let m: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(m["best"]["version"], "0.9.17");
    let out = rig.run(&["manifest", &rig.server.uri()]).await.success();
    let m: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(m["best"]["tenants"]["manifest"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_forgets_one_entry_and_nothing_else() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.run(&["logout", NAME])
        .await
        .success()
        .stderr(predicate::str::contains(format!("Forgot '{NAME}'")));
    let all = rig.credentials();
    assert!(all.get(NAME).is_none());
    assert_eq!(all["dotquant.io"], foreign_entry(&rig.server.uri()));
    rig.run(&["logout", NAME]).await.code(3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_surface_is_signed_in_without_a_credential() {
    let rig = Rig::new().await;
    let public = MockServer::start().await;
    let base = public.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "services": {"com.example.open": {"version": "1.0.0", "description": "Open", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&public)
        .await;
    Mock::given(method("GET"))
        .and(path("/queries"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"queries": [{"schema": "list-open", "version": "1.0", "dataschema": format!("{base}/queries/list-open/1.0")}]})))
        .mount(&public)
        .await;
    rig.run(&["login", "open", "--manifest", &base])
        .await
        .success()
        .stderr(predicate::str::contains("public"));
    assert_eq!(rig.credentials()["open"]["authType"], "none");
    rig.run(&["queries", "open"])
        .await
        .success()
        .stdout(predicate::str::contains("list-open"));
    let sent = public.received_requests().await.unwrap();
    assert!(
        sent.iter().all(|r| r.headers.get("x-api-key").is_none()),
        "no credential header on a public surface"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_credential_at_a_doors_root_opens_the_roots_services_and_heals_on_next_use() {
    let rig = Rig::new().await;
    let door = MockServer::start().await;
    let base = door.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "authentication": {"type": "apiKey", "scheme": "X-Api-Key", "in": "header", "tokenUrl": format!("{base}/auth/token"), "deviceAuthorizationUrl": format!("{base}/auth/device"), "note": "x"},
            "tenants": {"manifest": format!("{base}/.well-known/best/{{tenantId}}")},
            "services": {"io.best.agents": {"version": "1.0.0", "description": "The door", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&door)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code": "d", "user_code": "AAAA-BBBB", "verification_uri": format!("{base}/activate"), "expires_in": 60, "interval": 1})))
        .mount(&door)
        .await;
    // As best-peer answers for a credential at its root: tenant_id is the door's host, which names no workspace.
    let host = base.trim_start_matches("http://").to_owned();
    Mock::given(method("POST"))
        .and(path("/auth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": KEY, "token_type": "apikey", "auth_header": "X-Api-Key", "tenant_id": host, "name": "door", "endpoint": format!("{base}/")})))
        .mount(&door)
        .await;
    Mock::given(method("GET"))
        .and(path("/queries"))
        .and(header("X-Api-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"queries": [{"schema": "get-hardware-state", "version": "1.0", "dataschema": format!("{base}/queries/get-hardware-state/1.0")}]})))
        .mount(&door)
        .await;

    rig.run(&["login", "door", "--manifest", &base]).await.success();
    assert_eq!(
        rig.credentials()["door"]["endpoints"],
        json!({"io.best.agents": base.trim_end_matches('/')})
    );
    assert!(
        door.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().starts_with("/.well-known/best/")),
        "no tenant manifest was asked for"
    );

    // A sign-in that stored the credential but not the endpoints completes on its next use.
    let mut all = rig.credentials();
    all["door"]["endpoints"] = json!({});
    std::fs::write(rig.credentials_file(), serde_json::to_string_pretty(&all).unwrap()).unwrap();
    rig.run(&["queries", "door"])
        .await
        .success()
        .stdout(predicate::str::contains("get-hardware-state"));
    assert_eq!(
        rig.credentials()["door"]["endpoints"],
        json!({"io.best.agents": base.trim_end_matches('/')})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_door_that_publishes_no_guidance_prints_none() {
    let rig = Rig::new().await;
    let door = MockServer::start().await;
    let base = door.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "authentication": {"type": "apiKey", "scheme": "X-Api-Key", "in": "header", "tokenUrl": format!("{base}/auth/token"), "deviceAuthorizationUrl": format!("{base}/auth/device")},
            "services": {"com.example.x": {"version": "1.0.0", "description": "X", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&door)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code": "d", "user_code": "CCCC-DDDD", "verification_uri": format!("{base}/activate"), "expires_in": 60, "interval": 1})))
        .mount(&door)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": KEY, "token_type": "apikey", "tenant_id": "t"})),
        )
        .mount(&door)
        .await;
    rig.run(&["login", "plain", "--manifest", &base])
        .await
        .success()
        .stderr(predicate::str::contains("The code is CCCC-DDDD"))
        .stderr(predicate::str::contains("sign-in guidance").not());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_code_the_person_holds_is_redeemed_at_the_token_endpoint() {
    let rig = Rig::new().await;
    let door = MockServer::start().await;
    let base = door.uri();
    // Only a token endpoint: the authorization URL is not reachable from here, or not declared.
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "authentication": {"type": "apiKey", "scheme": "X-Api-Key", "in": "header", "tokenUrl": format!("{base}/auth/token")},
            "services": {"com.example.x": {"version": "1.0.0", "description": "X", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&door)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/token"))
        .and(body_string_contains("device_code=held-code-1"))
        .and(body_string_contains("client_id=best-cli"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": KEY, "token_type": "apikey", "auth_header": "X-Api-Key", "tenant_id": "t1", "name": "held"})))
        .mount(&door)
        .await;

    let out = rig
        .run(&["login", "held", "--manifest", &base, "--device-code", "held-code-1"])
        .await
        .success();
    assert_never_shows_key(&out);
    out.stderr(predicate::str::contains("Redeeming the device code at"))
        .stderr(predicate::str::contains("Signed in as 'held' (tenant t1)"))
        .stderr(predicate::str::contains("held-code-1").not())
        .stderr(predicate::str::contains("shell history"));
    let entry = &rig.credentials()["held"];
    assert_eq!(entry["apiKey"], KEY);
    assert_eq!(entry["tenantId"], "t1");
    assert_eq!(entry["manifest"], format!("{base}/.well-known/best"));
    assert!(
        door.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/auth/device"),
        "no sign-in was started"
    );

    // From stdin, so the code stays out of the process list.
    rig.run_with_stdin(
        &["login", "held2", "--manifest", &base, "--device-code", "-"],
        "held-code-1\n",
    )
    .await
    .success();
    assert_eq!(rig.credentials()["held2"]["apiKey"], KEY);
    rig.run_with_stdin(&["login", "held3", "--manifest", &base, "--device-code", "-"], "\n")
        .await
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_code_is_refused_at_once_by_a_door_with_no_token_endpoint() {
    let rig = Rig::new().await;
    let door = MockServer::start().await;
    let base = door.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "authentication": {"type": "apiKey", "scheme": "X-Api-Key", "in": "header"},
            "services": {"com.example.x": {"version": "1.0.0", "description": "X", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&door)
        .await;
    rig.run(&["login", "nope", "--manifest", &base, "--device-code", "abc"])
        .await
        .code(1)
        .stderr(predicate::str::contains("declares no tokenUrl"))
        .stderr(predicate::str::contains("abc").not());
    assert!(rig.credentials().get("nope").is_none());
    assert_eq!(
        door.received_requests().await.unwrap().len(),
        1,
        "only the manifest was read"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_code_is_not_swallowed_by_a_public_door() {
    let rig = Rig::new().await;
    let door = MockServer::start().await;
    let base = door.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "services": {"com.example.open": {"version": "1.0.0", "description": "Open", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&door)
        .await;
    rig.run(&["login", "open", "--manifest", &base, "--device-code", "abc"])
        .await
        .code(1)
        .stderr(predicate::str::contains("declares no tokenUrl"));
    assert!(rig.credentials().get("open").is_none(), "no entry was stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_sign_in_ends_with_the_services_words_and_exit_3() {
    let rig = Rig::new().await;
    let denied = MockServer::start().await;
    let base = denied.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/best"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"best": {
            "version": "0.9.17",
            "authentication": {"type": "apiKey", "scheme": "X-Api-Key", "in": "header", "tokenUrl": format!("{base}/auth/token"), "deviceAuthorizationUrl": format!("{base}/auth/device"), "note": "x"},
            "services": {"com.example.x": {"version": "1.0.0", "description": "X", "http": {"endpoint": format!("{base}/")}}},
            "capabilities": []
        }})))
        .mount(&denied)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code": "d", "user_code": "AAAA-BBBB", "verification_uri": format!("{base}/activate"), "expires_in": 60, "interval": 1})))
        .mount(&denied)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": "access_denied", "error_description": "the approver said no"})),
        )
        .mount(&denied)
        .await;
    rig.run(&["login", "nope", "--manifest", &base])
        .await
        .code(3)
        .stderr(predicate::str::contains("AAAA-BBBB"))
        .stderr(predicate::str::contains("access_denied: the approver said no"));
    assert!(rig.credentials().get("nope").is_none(), "nothing was stored");
}

/// What `best watch` reads: two queries whose first answers are A, and whose later answers move as the test says.
async fn watched(rig: &Rig, later_things: Value) {
    rig.catalogues().await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .and(header("X-Api-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{"name": "first", "state": "up"}]})))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .and(header("X-Api-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(later_things))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/get-total")))
        .and(header("X-Api-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total": 7})))
        .mount(&rig.server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_shows_each_answer_then_the_events_and_an_answer_again_only_when_it_moved() {
    let rig = Rig::new().await;
    rig.signed_in();
    watched(&rig, json!({"items": [{"name": "first", "state": "busy"}]})).await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .and(header("Accept", "text/event-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(": keepalive\n\nid: ev1\ndata: {\"id\":\"ev1\",\"type\":\"ThingMoved\",\"correlationid\":\"c1\",\"time\":\"2026-10-22T10:00:00Z\",\"data\":{\"name\":\"first\"}}\n\n"),
        )
        .mount(&rig.server)
        .await;

    let out = rig
        .run(&["watch", NAME, "list-things", "get-total", "--limit", "2"])
        .await
        .success();
    assert_never_shows_key(&out);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let at = |needle: &str| {
        stdout
            .find(needle)
            .unwrap_or_else(|| panic!("no {needle:?} in:\n{stdout}"))
    };
    // The first answers, each under its query's name; the event as one line; then the answer that moved, alone.
    assert!(at("== list-things") < at("first  up"), "{stdout}");
    assert!(at("first  up") < at("== get-total"), "{stdout}");
    assert!(at("== get-total") < at("ThingMoved"), "{stdout}");
    assert!(at("ThingMoved") < at("first  busy"), "{stdout}");
    assert_eq!(stdout.matches("== list-things").count(), 2, "{stdout}");
    assert_eq!(
        stdout.matches("== get-total").count(),
        1,
        "an answer that did not move is not shown again: {stdout}"
    );

    // The stream was opened from now on, with no history: the answers are what is so now.
    let streams = rig.requests("GET", &rig.tenant("events/stream")).await;
    assert!(!streams.is_empty());
    assert!(streams[0].headers.get("Last-Event-ID").is_none());
    assert!(
        rig.requests("GET", &rig.tenant("events")).await.is_empty(),
        "no history is read"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_reads_the_answers_again_on_its_own_since_some_change_with_no_event() {
    let rig = Rig::new().await;
    rig.signed_in();
    watched(&rig, json!({"items": [{"name": "first", "state": "held"}]})).await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(": keepalive\n\n"),
        )
        .mount(&rig.server)
        .await;

    let out = rig
        .run(&["watch", NAME, "list-things", "--every", "1", "--limit", "1"])
        .await
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(stdout.find("up").unwrap() < stdout.find("held").unwrap(), "{stdout}");
    assert_eq!(stdout.matches("== list-things").count(), 2, "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_with_json_is_one_line_per_answer_or_event() {
    let rig = Rig::new().await;
    rig.signed_in();
    watched(&rig, json!({"items": [{"name": "first", "state": "busy"}]})).await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string("id: ev1\ndata: {\"id\":\"ev1\",\"type\":\"ThingMoved\"}\n\n"),
        )
        .mount(&rig.server)
        .await;

    let out = rig
        .run(&["watch", NAME, "list-things", "get-total", "--json", "--limit", "2"])
        .await
        .success();
    let lines: Vec<Value> = String::from_utf8_lossy(&out.get_output().stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0]["query"], "list-things");
    assert_eq!(lines[0]["answer"]["items"][0]["state"], "up");
    assert!(lines[0]["time"].as_str().unwrap().ends_with('Z'));
    assert_eq!(lines[1]["query"], "get-total");
    assert_eq!(lines[1]["answer"], json!({"total": 7}));
    assert_eq!(lines[2]["event"]["id"], "ev1");
    assert_eq!(lines[3]["query"], "list-things");
    assert_eq!(lines[3]["answer"]["items"][0]["state"], "busy");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_refuses_a_query_the_service_does_not_answer_before_it_reads_anything() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    rig.run(&["watch", NAME, "list-things", "no-such-query"])
        .await
        .failure()
        .stderr(predicate::str::contains("no-such-query"));
    assert!(rig.requests("GET", &rig.tenant("queries/list-things")).await.is_empty());
    assert!(rig.requests("GET", &rig.tenant("events/stream")).await.is_empty());
    rig.run(&["watch", NAME]).await.code(2);
    rig.run(&["watch", NAME, "list-things", "--every", "0"]).await.code(2);
}

/// One query whose answer never moves, so only what a test sends counts as an update.
async fn still(rig: &Rig) {
    rig.catalogues().await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .and(header("X-Api-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{"name": "first", "state": "up"}]})))
        .mount(&rig.server)
        .await;
}

fn sse(frames: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("Content-Type", "text/event-stream")
        .set_body_string(frames.to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_reopens_the_stream_after_the_last_event_even_while_its_answers_are_read_every_second() {
    let rig = Rig::new().await;
    rig.signed_in();
    still(&rig).await;
    // The first stream gives ev1 and closes; the reopened one, asked after ev1, replays it and gives ev2.
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .and(header("Last-Event-ID", "ev1"))
        .respond_with(sse("id: ev1\ndata: {\"id\":\"ev1\",\"type\":\"First\"}\n\nid: ev2\ndata: {\"id\":\"ev2\",\"type\":\"Second\"}\n\n"))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .respond_with(sse("id: ev1\ndata: {\"id\":\"ev1\",\"type\":\"First\"}\n\n"))
        .mount(&rig.server)
        .await;

    let out = rig
        .run(&["watch", NAME, "list-things", "--every", "1", "--limit", "2"])
        .await
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert_eq!(
        stdout.matches("First").count(),
        1,
        "ev1 once, though replayed: {stdout}"
    );
    assert!(stdout.contains("Second"), "{stdout}");
    assert_eq!(
        stdout.matches("== list-things").count(),
        1,
        "an answer that never moved is shown once: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_says_a_service_that_fails_for_a_moment_and_goes_on() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    for (status, body) in [
        (200, json!({"items": [{"name": "first", "state": "up"}]})),
        (503, json!({"error": {"code": "DOWN", "message": "restarting"}})),
    ] {
        Mock::given(method("GET"))
            .and(path(rig.tenant("queries/list-things")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .up_to_n_times(1)
            .mount(&rig.server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{"name": "first", "state": "up"}]})))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .respond_with(sse(": keepalive\n\n"))
        .mount(&rig.server)
        .await;

    let out = rig
        .run(&["watch", NAME, "list-things", "--every", "1", "--limit", "1"])
        .await
        .success()
        .stderr(predicate::str::contains("list-things: DOWN: restarting (HTTP 503)"));
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert_eq!(
        stdout.matches("== list-things").count(),
        2,
        "the same answer is shown again once it is back, since the failure stood between: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_ends_on_the_services_refusal_and_on_a_credential_it_no_longer_takes() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/get-total")))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": {"code": "MISSING_PARAMETER", "message": "brokerId is required"}})),
        )
        .mount(&rig.server)
        .await;
    rig.run(&["watch", NAME, "get-total"])
        .await
        .code(5)
        .stderr(predicate::str::contains("MISSING_PARAMETER: brokerId is required"));
    assert!(rig.requests("GET", &rig.tenant("events/stream")).await.is_empty());

    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{"name": "first", "state": "up"}]})))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("queries/list-things")))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": {"code": "UNAUTHORIZED", "message": "revoked"}})),
        )
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .respond_with(sse(": keepalive\n\n"))
        .mount(&rig.server)
        .await;
    let out = rig.run(&["watch", NAME, "list-things", "--every", "1"]).await.code(3);
    assert_never_shows_key(&out);
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_stops_at_its_limit_even_when_several_answers_moved_at_once() {
    let rig = Rig::new().await;
    rig.signed_in();
    rig.catalogues().await;
    // Both answers move between the first read and the next.
    for (query, first, then) in [
        ("get-total", json!({"total": 7}), json!({"total": 8})),
        (
            "list-things",
            json!({"items": [{"name": "first", "state": "up"}]}),
            json!({"items": [{"name": "first", "state": "busy"}]}),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(rig.tenant(&format!("queries/{query}"))))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .up_to_n_times(1)
            .mount(&rig.server)
            .await;
        Mock::given(method("GET"))
            .and(path(rig.tenant(&format!("queries/{query}"))))
            .respond_with(ResponseTemplate::new(200).set_body_json(then))
            .mount(&rig.server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(rig.tenant("events/stream")))
        .respond_with(sse(": keepalive\n\n"))
        .mount(&rig.server)
        .await;

    let out = rig
        .run(&[
            "watch",
            NAME,
            "get-total",
            "list-things",
            "--every",
            "1",
            "--limit",
            "1",
            "--json",
        ])
        .await
        .success();
    let lines: Vec<Value> = String::from_utf8_lossy(&out.get_output().stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        lines.len(),
        3,
        "the two first answers, then one update and no more: {lines:?}"
    );
    assert_eq!(lines[2]["query"], "get-total");
    assert_eq!(lines[2]["answer"]["total"], 8);
}
