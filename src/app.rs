//! The commands as the person runs them, each on top of the modules: nothing here knows one service from another.

use std::io::{IsTerminal, Read};

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use url::Url;

use crate::catalogue::{self, Entry as CatalogueEntry, Kind};
use crate::cli::{Cli, Command, EventsArgs, Kind as SchemaKind, LoginArgs, SendArgs};
use crate::credentials::{self, Auth, Entry, Store};
use crate::http::{join, Http};
use crate::manifest::{self, Manifest};
use crate::output::{self, NotSignedIn, Usage};
use crate::say;
use crate::signin;

/// The `source` of every command this program sends: its own identity, never a routing key.
pub const SOURCE: &str = "urn:best-cli";

struct Ctx {
    http: Http,
    json: bool,
}

/// A connection as a command uses it: its entry, and the one endpoint the name (or `name/service`) picks.
struct Connection {
    name: String,
    service: String,
    endpoint: String,
    auth: Auth,
}

pub async fn run(cli: Cli) -> Result<()> {
    let ctx = Ctx {
        http: Http::new(cli.global.ca_file.as_deref())?,
        json: cli.global.json,
    };
    match cli.command {
        Command::Login(args) => login(&ctx, args).await,
        Command::Logout { name } => logout(&ctx, &name),
        Command::Connections => connections(&ctx),
        Command::Commands { name, filter } => list(&ctx, &name, Kind::Commands, filter.as_deref()).await,
        Command::Queries { name, filter } => list(&ctx, &name, Kind::Queries, filter.as_deref()).await,
        Command::Schema {
            name,
            schema: wanted,
            kind,
        } => schema(&ctx, &name, &wanted, kind).await,
        Command::Query { name, schema, params } => query(&ctx, &name, &schema, &params).await,
        Command::Send(args) => send(&ctx, args).await,
        Command::Events(args) => events(&ctx, args).await,
        Command::Workflows { name, id } => workflows(&ctx, &name, id.as_deref()).await,
        Command::Manifest { target } => show_manifest(&ctx, &target).await,
    }
}

// ── connections ──────────────────────────────────────────────────────────────

/// `name`, or `name/<service id>` for a connection whose manifest names several services. A sign-in whose
/// endpoints could not be resolved at the end of the exchange completes here, on its next use: nobody approves
/// again.
async fn connection(ctx: &Ctx, name: &str) -> Result<Connection> {
    let mut store = Store::open()?;
    let (bare, service) = match name.split_once('/') {
        Some((n, s)) if !s.is_empty() => (n, Some(s)),
        _ => (name, None),
    };
    let Some(mut entry) = store.get(bare) else {
        bail!(NotSignedIn(format!(
            "no connection named '{bare}': `best connections` lists them, `best login {bare} --manifest <address>` signs in"
        )));
    };
    if entry.endpoints.is_empty() {
        let Some(address) = entry.manifest.clone() else {
            bail!(NotSignedIn(format!(
                "'{bare}' holds a credential but no endpoint and no manifest: `best logout {bare}`, then sign in again"
            )));
        };
        let root = manifest::resolve(&ctx.http, &address).await?;
        entry.endpoints = manifest::endpoints_for(&ctx.http, &root, entry.tenant_id.as_deref(), &entry.auth)
            .await
            .with_context(|| format!("'{bare}' is signed in but its endpoints could not be resolved"))?;
        store.insert(bare, &entry)?;
    }
    let (service, endpoint) = entry.endpoint(service)?;
    Ok(Connection {
        name: bare.to_owned(),
        service,
        endpoint,
        auth: entry.auth,
    })
}

fn connections(ctx: &Ctx) -> Result<()> {
    let store = Store::open()?;
    let all = store.all();
    if ctx.json {
        let listed: Vec<Value> = all
            .iter()
            .map(|(name, e)| {
                json!({
                    "name": name,
                    "tenantId": e.tenant_id,
                    "manifest": e.manifest,
                    "endpoints": endpoints_json(&e.endpoints),
                    "issuedAt": e.issued_at,
                    "auth": match &e.auth { Auth::None => "none", Auth::Bearer(_) => "bearer", Auth::ApiKey { .. } => "apikey" },
                })
            })
            .collect();
        return output::json(&listed);
    }
    if all.is_empty() {
        say!(
            "No connections in {}. Sign in with `best login <name> --manifest <address>`.",
            store.path().display()
        );
        return Ok(());
    }
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|(name, e)| {
            vec![
                name.clone(),
                e.host(),
                e.tenant_id.clone().unwrap_or_default(),
                e.endpoints
                    .iter()
                    .map(|(k, _)| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                e.issued_at
                    .as_deref()
                    .map(|t| t.chars().take(10).collect())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    output::table(&["NAME", "HOST", "TENANT", "SERVICES", "SIGNED IN"], &rows)
}

// ── login / logout ───────────────────────────────────────────────────────────

async fn login(ctx: &Ctx, args: LoginArgs) -> Result<()> {
    let name = args.name.trim().to_owned();
    if !credentials::valid_name(&name) {
        bail!(Usage(format!(
            "'{name}' cannot be a connection name: letters, digits, '.', '_' and '-', up to 64, starting with a letter or digit"
        )));
    }
    let mut store = Store::open()?;
    if let Some(existing) = store.get(&name) {
        bail!(
            "'{name}' is already signed in at {} ({}). It is left as it is: `best logout {name}` first, or choose another name.",
            existing.manifest.as_deref().unwrap_or("an endpoint"),
            store.path().display()
        );
    }

    let root = manifest::resolve(&ctx.http, &args.manifest).await?;
    say!("Resolved {} to {}", args.manifest.trim(), root.url);

    if root.is_public() && root.registration()?.is_none() {
        let endpoints = manifest::endpoints_for(&ctx.http, &root, None, &Auth::None).await?;
        let entry = Entry {
            tenant_id: None,
            auth: Auth::None,
            issued_at: Some(now()),
            manifest: Some(root.url.to_string()),
            endpoints,
        };
        store.insert(&name, &entry)?;
        return report_signed_in(ctx, &store, &name, &entry, "no credential: the surface is public");
    }

    let started = match &args.device_code {
        Some(code) => {
            let code = device_code(code)?;
            let Some(token_url) = root.token_url()? else {
                bail!(
                    "{} declares no tokenUrl, so there is no endpoint to redeem a device code at. Sign in where its manifest says, or without --device-code if it declares a deviceAuthorizationUrl.",
                    root.host()
                );
            };
            say!("Redeeming the device code at {token_url}…");
            signin::held(&token_url, code)
        }
        None => {
            let Some(registration) = root.registration()? else {
                bail!(
                    "{} declares no deviceAuthorizationUrl: this service issues credentials out of band, on its own site. best-cli cannot sign you in there. If you hold a device code, redeem it with --device-code.",
                    root.host()
                );
            };
            let label = args
                .label
                .clone()
                .unwrap_or_else(|| format!("best-cli on {}", hostname()));
            let started = signin::start(&ctx.http, &registration, &credentials::connection_word(&name), &label).await?;

            say!();
            say!("Open this link in your own browser, sign in and approve:");
            say!();
            say!(
                "    {}",
                started
                    .verification_uri_complete
                    .as_deref()
                    .unwrap_or(&started.verification_uri)
            );
            say!();
            say!(
                "The code is {}. Waiting for your approval (the code expires in {} minutes)…",
                started.user_code,
                started.expires_in.div_ceil(60)
            );
            guidance(registration.note.as_deref(), started.note.as_deref());
            started
        }
    };

    let issued = tokio::select! {
        r = signin::wait(&ctx.http, &started, || {}) => r?,
        _ = tokio::signal::ctrl_c() => bail!(output::NotConfirmed("sign-in cancelled".into())),
    };

    let auth = if issued.token_type.eq_ignore_ascii_case("bearer") {
        Auth::Bearer(issued.access_token.clone())
    } else {
        Auth::ApiKey {
            header: issued.auth_header.clone().unwrap_or_else(|| "X-Api-Key".into()),
            key: issued.access_token.clone(),
        }
    };
    // The device code is spent: the credential is kept before anything else can fail, so a failed discovery
    // below costs a retry, never a second approval.
    let mut entry = Entry {
        tenant_id: issued.tenant_id.clone(),
        auth,
        issued_at: Some(now()),
        manifest: Some(root.url.to_string()),
        endpoints: Vec::new(),
    };
    store.insert(&name, &entry)?;
    match manifest::endpoints_for(&ctx.http, &root, issued.tenant_id.as_deref(), &entry.auth).await {
        Ok(endpoints) => {
            entry.endpoints = endpoints;
            store.insert(&name, &entry)?;
        }
        Err(e) => bail!(
            "'{name}' is signed in (the credential is stored in {}) but its endpoints could not be resolved: {e:#}. The next `best queries {name}` tries again; nothing needs approving again.",
            store.path().display()
        ),
    }
    let how = match issued.expires_in {
        Some(s) => format!("the credential expires in {s} seconds"),
        None => "the credential is durable until revoked".into(),
    };
    report_signed_in(ctx, &store, &name, &entry, &how)
}

/// The service's own sign-in guidance, exactly as its manifest and its device answer carry it (SPEC Sign-in
/// Guidance): what the person reads to know what approving does. Nothing when the service published none.
fn guidance(manifest_note: Option<&str>, device_note: Option<&str>) {
    let notes: Vec<&str> = [manifest_note, device_note].into_iter().flatten().collect();
    if notes.is_empty() {
        return;
    }
    say!();
    say!("The service's sign-in guidance, as it wrote it:");
    for note in notes {
        say!();
        say!("{note}");
    }
}

/// `--device-code`: the code, or `-` to read it from stdin.
fn device_code(given: &str) -> Result<String> {
    let code = if given == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("reading the device code from stdin")?;
        s
    } else {
        given.to_owned()
    };
    let code = code.trim().to_owned();
    if code.is_empty() {
        bail!(Usage("--device-code is empty".into()));
    }
    Ok(code)
}

fn report_signed_in(ctx: &Ctx, store: &Store, name: &str, entry: &Entry, how: &str) -> Result<()> {
    if ctx.json {
        return output::json(&json!({
            "name": name,
            "tenantId": entry.tenant_id,
            "manifest": entry.manifest,
            "endpoints": endpoints_json(&entry.endpoints),
            "storedAt": store.path(),
        }));
    }
    say!();
    say!(
        "Signed in as '{name}'{} at {}; {how}.",
        entry
            .tenant_id
            .as_deref()
            .map(|t| format!(" (tenant {t})"))
            .unwrap_or_default(),
        entry.host()
    );
    say!(
        "The credential is in {}, where best-mcp finds it too. It is never printed.",
        store.path().display()
    );
    for (service, endpoint) in &entry.endpoints {
        say!("  {service}  {endpoint}");
    }
    say!("Next: `best queries {name}` and `best commands {name}`.");
    Ok(())
}

fn logout(ctx: &Ctx, name: &str) -> Result<()> {
    let mut store = Store::open()?;
    if !store.remove(name)? {
        bail!(NotSignedIn(format!(
            "no connection named '{name}' in {}",
            store.path().display()
        )));
    }
    if ctx.json {
        return output::json(&json!({"name": name, "removed": true}));
    }
    say!("Forgot '{name}'. The credential itself stays valid until the service revokes it (its catalogue says how).");
    Ok(())
}

// ── catalogues and schemas ───────────────────────────────────────────────────

async fn list(ctx: &Ctx, name: &str, kind: Kind, filter: Option<&str>) -> Result<()> {
    let c = connection(ctx, name).await?;
    let entries = catalogue::fetch(&ctx.http, &c.endpoint, &c.auth, kind).await?;
    let shown: Vec<&CatalogueEntry> = entries.iter().filter(|e| catalogue::matches(e, filter)).collect();
    if ctx.json {
        return output::json(&shown.iter().map(|e| e.raw.clone()).collect::<Vec<_>>());
    }
    if shown.is_empty() {
        say!(
            "{} {}",
            c.service,
            match filter {
                Some(f) => format!("has no {} matching '{f}'", kind.path()),
                None => format!("publishes no {}", kind.path()),
            }
        );
        return Ok(());
    }
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|e| {
            let mut row = vec![e.schema.clone(), e.version.clone()];
            if kind == Kind::Commands {
                row.push(
                    e.impact
                        .as_ref()
                        .map(|i| format!("{} ({})", i.categories.join(","), i.confirmation))
                        .unwrap_or_default(),
                );
            }
            row.push(output::summary(e.description.as_deref(), 90));
            row
        })
        .collect();
    match kind {
        Kind::Commands => output::table(&["COMMAND", "VERSION", "IMPACT", "DESCRIPTION"], &rows),
        Kind::Queries => output::table(&["QUERY", "VERSION", "DESCRIPTION"], &rows),
    }
}

async fn schema(ctx: &Ctx, name: &str, wanted: &str, kind: Option<SchemaKind>) -> Result<()> {
    let c = connection(ctx, name).await?;
    let _ = ctx;
    let url = match kind {
        Some(SchemaKind::Event) => {
            let (schema, version) = catalogue::split_version(wanted);
            let Some(version) = version else {
                bail!(Usage(format!(
                    "an event schema needs its version: `best schema {name} {schema}/<version> --kind event`"
                )));
            };
            join(&c.endpoint, &format!("events/{schema}/{version}"))?
        }
        Some(SchemaKind::Command) => catalogue_schema_url(ctx, &c, wanted, Kind::Commands).await?,
        Some(SchemaKind::Query) => catalogue_schema_url(ctx, &c, wanted, Kind::Queries).await?,
        None => {
            let commands = catalogue::fetch(&ctx.http, &c.endpoint, &c.auth, Kind::Commands).await?;
            let queries = catalogue::fetch(&ctx.http, &c.endpoint, &c.auth, Kind::Queries)
                .await
                .unwrap_or_default();
            let command = catalogue::find(&commands, wanted, Kind::Commands).ok();
            let query = catalogue::find(&queries, wanted, Kind::Queries).ok();
            match (command, query) {
                (Some(_), Some(_)) => bail!(Usage(format!(
                    "'{wanted}' is both a command and a query here: say which with --kind command or --kind query"
                ))),
                (Some(e), None) => join(&c.endpoint, &format!("commands/{}/{}", e.schema, e.version))?,
                (None, Some(e)) => join(&c.endpoint, &format!("queries/{}/{}", e.schema, e.version))?,
                (None, None) => bail!(
                    "the service has no command or query called '{wanted}' (an event needs --kind event and its version): `best commands {name}` and `best queries {name}` list them"
                ),
            }
        }
    };
    let document = ctx.http.get_json(&url, &c.auth).await?;
    output::json(&document)
}

async fn catalogue_schema_url(ctx: &Ctx, c: &Connection, wanted: &str, kind: Kind) -> Result<Url> {
    let entries = catalogue::fetch(&ctx.http, &c.endpoint, &c.auth, kind).await?;
    let entry = catalogue::find(&entries, wanted, kind)?;
    join(
        &c.endpoint,
        &format!("{}/{}/{}", kind.path(), entry.schema, entry.version),
    )
}

// ── queries ──────────────────────────────────────────────────────────────────

async fn query(ctx: &Ctx, name: &str, wanted: &str, params: &[String]) -> Result<()> {
    let c = connection(ctx, name).await?;
    let pairs = params
        .iter()
        .map(|p| {
            p.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or_else(|| Usage(format!("'{p}': a parameter is NAME=VALUE")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let entries = catalogue::fetch(&ctx.http, &c.endpoint, &c.auth, Kind::Queries).await?;
    let entry = catalogue::find(&entries, wanted, Kind::Queries)?;
    let mut url = join(&c.endpoint, &format!("queries/{}", entry.schema))?;
    if !pairs.is_empty() {
        url.query_pairs_mut()
            .extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    }
    let answer = ctx.http.get_json(&url, &c.auth).await?;
    if ctx.json {
        output::json(&answer)
    } else {
        output::answer(&answer)
    }
}

// ── commands ─────────────────────────────────────────────────────────────────

async fn send(ctx: &Ctx, args: SendArgs) -> Result<()> {
    let c = connection(ctx, &args.name).await?;
    let data = payload(&args)?;
    let entries = catalogue::fetch(&ctx.http, &c.endpoint, &c.auth, Kind::Commands).await?;
    let entry = catalogue::find(&entries, &args.schema, Kind::Commands)?;
    let kind = entry.envelope_type();

    // The envelope, as best-mcp builds it: the type the catalogue states, the dataschema on this surface, this
    // program as the source, a fresh id, and the correlation id only when the person joins an existing chain.
    let mut envelope = Map::new();
    envelope.insert("specversion".into(), json!("1.0"));
    envelope.insert(
        "id".into(),
        json!(args.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string())),
    );
    envelope.insert(
        "source".into(),
        json!(args.source.clone().unwrap_or_else(|| SOURCE.to_owned())),
    );
    envelope.insert("type".into(), json!(kind));
    envelope.insert("datacontenttype".into(), json!("application/json"));
    envelope.insert(
        "dataschema".into(),
        json!(format!(
            "{}/commands/{}/{}",
            c.endpoint.trim_end_matches('/'),
            entry.schema,
            entry.version
        )),
    );
    envelope.insert("time".into(), json!(now()));
    if let Some(correlation) = &args.correlation_id {
        envelope.insert("correlationid".into(), json!(correlation));
    }
    envelope.insert("data".into(), data);
    let envelope = Value::Object(envelope);
    let url = join(&c.endpoint, "commands")?;

    if args.dry_run {
        output::json(&json!({"post": url.as_str(), "body": envelope}))?;
        say!("Dry run: nothing was sent.");
        return Ok(());
    }

    if let Some(impact) = &entry.impact {
        say!(
            "{} is a high-impact command ({}; confirmation {}).",
            entry.schema,
            impact.categories.join(", "),
            impact.confirmation
        );
        if let Some(w) = &impact.warning {
            say!("Warning: {w}");
        }
        output::confirm(&format!("Send {kind} to {} ({})?", c.service, c.name), args.yes)?;
    }

    let accepted = ctx.http.post_command(&url, &c.auth, &envelope).await?;
    if ctx.json {
        return output::json(&accepted);
    }
    let id = accepted["id"].as_str().unwrap_or("");
    let correlation = accepted["correlationId"]
        .as_str()
        .or(args.correlation_id.as_deref())
        .unwrap_or(id);
    say!("Accepted {kind}: id {id}, correlation {correlation}.");
    say!(
        "Follow it with `best events {} --correlation-id {correlation} --follow`.",
        c.name
    );
    Ok(())
}

/// `--data` (JSON, or `-` for stdin), `--file`, or `{}`.
fn payload(args: &SendArgs) -> Result<Value> {
    let text = match (&args.data, &args.file) {
        (Some(d), _) if d == "-" => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading the payload from stdin")?;
            s
        }
        (Some(d), _) => d.clone(),
        (None, Some(path)) => std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
        (None, None) => "{}".into(),
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| Usage(format!("the payload is not JSON: {e}")))?;
    if !value.is_object() {
        bail!(Usage("the payload must be a JSON object".into()));
    }
    Ok(value)
}

// ── events ───────────────────────────────────────────────────────────────────

async fn events(ctx: &Ctx, args: EventsArgs) -> Result<()> {
    let c = connection(ctx, &args.name).await?;
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(v) = &args.kind {
        pairs.push(("type", v.clone()));
    }
    if let Some(v) = &args.correlation_id {
        pairs.push(("correlationId", v.clone()));
    }
    if let Some(v) = &args.source {
        pairs.push(("source", v.clone()));
    }
    if args.follow {
        return follow(ctx, &c, &pairs, args.limit, args.last_event_id.as_deref()).await;
    }
    if let Some(v) = &args.from {
        pairs.push(("from", v.clone()));
    }
    if let Some(v) = &args.to {
        pairs.push(("to", v.clone()));
    }
    if let Some(v) = args.limit {
        pairs.push(("limit", v.to_string()));
    }
    if let Some(v) = &args.after {
        pairs.push(("after", v.clone()));
    }
    let mut url = join(&c.endpoint, "events")?;
    if !pairs.is_empty() {
        url.query_pairs_mut()
            .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
    }
    let answer = ctx.http.get_json(&url, &c.auth).await?;
    if ctx.json {
        return output::json(&answer);
    }
    let Some(list) = answer["events"].as_array() else {
        return output::answer(&answer);
    };
    if list.is_empty() {
        say!("No events.");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = list.iter().map(event_row).collect();
    output::table(&["TIME", "TYPE", "CORRELATION", "ID", "DATA"], &rows)?;
    if let Some(cursor) = answer["nextCursor"].as_str() {
        say!("More: --after {cursor}");
    }
    Ok(())
}

fn event_row(event: &Value) -> Vec<String> {
    vec![
        event["time"].as_str().unwrap_or("").to_owned(),
        event["type"].as_str().unwrap_or("").to_owned(),
        event["correlationid"]
            .as_str()
            .or(event["correlationId"].as_str())
            .unwrap_or("")
            .to_owned(),
        event["id"].as_str().unwrap_or("").to_owned(),
        output::cell(&event["data"]),
    ]
}

/// What happened, then what happens next (SPEC Events: load history first, then open the stream): the history page
/// is printed, the stream is opened after its last event (`Last-Event-ID`) and reopened the same way when the service
/// closes it, until Ctrl-C or `--limit` events in all. An id already shown is not shown again.
async fn follow(
    ctx: &Ctx,
    c: &Connection,
    pairs: &[(&str, String)],
    limit: Option<u32>,
    last_event_id: Option<&str>,
) -> Result<()> {
    let as_lines = ctx.json || !std::io::stdout().is_terminal();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut last_id = last_event_id.map(str::to_owned);
    let mut shown: u32 = 0;
    let done = |shown: u32| limit.is_some_and(|l| shown >= l);

    if last_event_id.is_none() {
        let mut history = join(&c.endpoint, "events")?;
        history
            .query_pairs_mut()
            .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
        if let Some(l) = limit {
            history.query_pairs_mut().append_pair("limit", &l.to_string());
        }
        let answer = ctx.http.get_json(&history, &c.auth).await?;
        for event in answer["events"].as_array().into_iter().flatten() {
            print_event(event, as_lines)?;
            if let Some(id) = event["id"].as_str() {
                seen.insert(id.to_owned());
                last_id = Some(id.to_owned());
            }
            shown += 1;
            if done(shown) {
                return Ok(());
            }
        }
    }

    let mut url = join(&c.endpoint, "events/stream")?;
    if !pairs.is_empty() {
        url.query_pairs_mut()
            .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
    }
    let mut first = true;
    loop {
        if !first {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        first = false;
        let response = ctx.http.open_stream(&url, &c.auth, last_id.as_deref()).await?;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        loop {
            let chunk = tokio::select! {
                chunk = stream.next() => chunk,
                _ = tokio::signal::ctrl_c() => return Ok(()),
            };
            match chunk {
                Some(Ok(bytes)) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
                Some(Err(e)) => {
                    say!("stream interrupted ({e}); reconnecting");
                    break;
                }
                None => break,
            }
            while let Some(frame_end) = buffer.find("\n\n") {
                let frame = buffer[..frame_end].to_owned();
                buffer.drain(..frame_end + 2);
                let (id, data) = parse_frame(&frame);
                let Some(data) = data else { continue };
                let event: Value = serde_json::from_str(&data).unwrap_or(Value::String(data.clone()));
                let event_id = id.or_else(|| event["id"].as_str().map(str::to_owned));
                if let Some(event_id) = &event_id {
                    last_id = Some(event_id.clone());
                    if !seen.insert(event_id.clone()) {
                        continue;
                    }
                }
                print_event(&event, as_lines)?;
                shown += 1;
                if done(shown) {
                    return Ok(());
                }
            }
        }
    }
}

/// An SSE frame's `id` and joined `data` lines; comments (keepalives) are skipped.
fn parse_frame(frame: &str) -> (Option<String>, Option<String>) {
    let mut id = None;
    let mut data: Vec<&str> = Vec::new();
    for raw in frame.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with(':') {
            continue;
        }
        if let Some(d) = line.strip_prefix("data:") {
            data.push(d.strip_prefix(' ').unwrap_or(d));
        } else if let Some(i) = line.strip_prefix("id:") {
            id = Some(i.trim().to_owned());
        }
    }
    (id, if data.is_empty() { None } else { Some(data.join("\n")) })
}

fn print_event(event: &Value, as_lines: bool) -> Result<()> {
    if as_lines {
        println!("{event}");
    } else {
        let row = event_row(event);
        println!("{}  {}  {}  {}", row[0], row[1], row[2], row[4]);
    }
    Ok(())
}

// ── workflows ────────────────────────────────────────────────────────────────

async fn workflows(ctx: &Ctx, name: &str, id: Option<&str>) -> Result<()> {
    let c = connection(ctx, name).await?;
    match id {
        None => {
            let answer = ctx.http.get_json(&join(&c.endpoint, "workflows")?, &c.auth).await?;
            if ctx.json {
                return output::json(&answer);
            }
            let list = answer["workflows"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                say!("{} publishes no workflows.", c.service);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|w| {
                    vec![
                        w["id"].as_str().unwrap_or("").to_owned(),
                        w["name"].as_str().unwrap_or("").to_owned(),
                        output::summary(w["description"].as_str(), 90),
                    ]
                })
                .collect();
            output::table(&["ID", "NAME", "DESCRIPTION"], &rows)
        }
        Some(id) => {
            let answer = ctx
                .http
                .get_json(&join(&c.endpoint, &format!("workflows/{id}"))?, &c.auth)
                .await?;
            if ctx.json {
                return output::json(&answer);
            }
            println!(
                "{}\n\n{}\n",
                answer["name"].as_str().unwrap_or(id),
                answer["description"].as_str().unwrap_or("")
            );
            for (i, step) in answer["steps"].as_array().into_iter().flatten().enumerate() {
                let optional = if step["optional"].as_bool() == Some(true) {
                    " (optional)"
                } else {
                    ""
                };
                println!(
                    "{}. {} {}{optional}",
                    i + 1,
                    step["kind"].as_str().unwrap_or(""),
                    schema_of(step["dataschema"].as_str().unwrap_or(""))
                );
                if let Some(g) = step["guidance"].as_str() {
                    println!("   {g}");
                }
            }
            Ok(())
        }
    }
}

/// `https://…/commands/deploy-model/1.0` → `deploy-model/1.0`.
fn schema_of(dataschema: &str) -> String {
    let parts: Vec<&str> = dataschema.trim_end_matches('/').rsplit('/').take(2).collect();
    match parts.as_slice() {
        [version, name] => format!("{name}/{version}"),
        _ => dataschema.to_owned(),
    }
}

// ── manifest ─────────────────────────────────────────────────────────────────

async fn show_manifest(ctx: &Ctx, target: &str) -> Result<()> {
    let store = Store::open()?;
    let m: Manifest = match store.get(target).and_then(|e| e.manifest) {
        Some(address) => manifest::resolve(&ctx.http, &address).await?,
        None => manifest::resolve(&ctx.http, target).await?,
    };
    if !ctx.json {
        say!("{}", m.url);
    }
    output::json(&m.value)
}

// ── small things ─────────────────────────────────────────────────────────────

fn endpoints_json(endpoints: &[(String, String)]) -> Value {
    Value::Object(
        endpoints
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    )
}

fn now() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_owned())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "this machine".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_frames_yield_id_and_data() {
        let (id, data) = parse_frame("id: 7\ndata: {\"a\":1}\ndata: more");
        assert_eq!(id.as_deref(), Some("7"));
        assert_eq!(data.as_deref(), Some("{\"a\":1}\nmore"));
        assert_eq!(parse_frame(": keepalive"), (None, None));
        assert_eq!(parse_frame("data:x\r"), (None, Some("x".into())));
    }

    #[test]
    fn a_schema_is_named_from_its_url() {
        assert_eq!(
            schema_of("https://x/api/best/tenants/t/commands/submit-expense/2.0"),
            "submit-expense/2.0"
        );
    }

    #[test]
    fn times_are_rfc3339_utc() {
        let t = now();
        assert!(t.ends_with('Z') && t.len() == 24, "{t}");
    }
}
