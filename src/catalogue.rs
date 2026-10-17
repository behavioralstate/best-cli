//! The command and query catalogues, as the service publishes them, and how a name on the command line finds an
//! entry: by schema name or type, with or without a version, the latest version when none is said.

use anyhow::{bail, Result};
use serde_json::Value;

use crate::credentials::Auth;
use crate::http::{join, Http};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commands,
    Queries,
}

impl Kind {
    pub fn path(self) -> &'static str {
        match self {
            Kind::Commands => "commands",
            Kind::Queries => "queries",
        }
    }

    fn singular(self) -> &'static str {
        match self {
            Kind::Commands => "command",
            Kind::Queries => "query",
        }
    }
}

/// The high-impact annotation (SPEC Impact Annotations).
#[derive(Debug, Clone)]
pub struct Impact {
    pub categories: Vec<String>,
    pub confirmation: String,
    pub warning: Option<String>,
}

/// One catalogue entry, with the whole document kept for `--json`.
#[derive(Debug, Clone)]
pub struct Entry {
    pub schema: String,
    pub version: String,
    pub command_type: Option<String>,
    pub description: Option<String>,
    pub impact: Option<Impact>,
    pub raw: Value,
}

impl Entry {
    fn from_value(value: &Value) -> Option<Self> {
        let schema = value["schema"].as_str()?.to_owned();
        let impact = value["impact"].as_object().map(|i| Impact {
            categories: i
                .get("categories")
                .and_then(Value::as_array)
                .map(|c| c.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default(),
            // An unknown or absent level is the strict one.
            confirmation: match i.get("confirmation").and_then(Value::as_str) {
                Some("recommended") => "recommended".into(),
                _ => "required".into(),
            },
            warning: i.get("warning").and_then(Value::as_str).map(str::to_owned),
        });
        Some(Self {
            schema,
            version: value["version"].as_str().unwrap_or("").to_owned(),
            command_type: value["commandType"].as_str().map(str::to_owned),
            description: value["description"].as_str().map(str::to_owned),
            impact,
            raw: value.clone(),
        })
    }

    /// The envelope `type`: what the catalogue states, else the PascalCase of the schema name (a guess the
    /// spec allows when the service states none).
    pub fn envelope_type(&self) -> String {
        match &self.command_type {
            Some(t) => t.clone(),
            None => self
                .schema
                .split('-')
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().chain(c).collect::<String>())
                        .unwrap_or_default()
                })
                .collect(),
        }
    }
}

/// `GET /commands` or `GET /queries`.
pub async fn fetch(http: &Http, endpoint: &str, auth: &Auth, kind: Kind) -> Result<Vec<Entry>> {
    let url = join(endpoint, kind.path())?;
    let value = http.get_json(&url, auth).await?;
    let entries = value[kind.path()]
        .as_array()
        .map(|a| a.iter().filter_map(Entry::from_value).collect::<Vec<_>>());
    match entries {
        Some(e) => Ok(e),
        None => bail!("{url} answered without a \"{}\" list: {}", kind.path(), value),
    }
}

/// Lower case without hyphens or underscores: `deploy-model`, `DeployModel` and `deploymodel` are one name.
fn normalise(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// `deploy-model/1.0` → (`deploy-model`, Some(`1.0`)).
pub fn split_version(name: &str) -> (&str, Option<&str>) {
    match name.split_once('/') {
        Some((n, v)) if !v.is_empty() => (n, Some(v)),
        _ => (name, None),
    }
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a).cmp(&parts(b))
}

/// Finds an entry by schema name or type and optional version. A miss names the near entries.
pub fn find<'a>(entries: &'a [Entry], name: &str, kind: Kind) -> Result<&'a Entry> {
    let (bare, version) = split_version(name);
    let key = normalise(bare);
    let found: Vec<&Entry> = entries
        .iter()
        .filter(|e| normalise(&e.schema) == key || e.command_type.as_deref().is_some_and(|t| normalise(t) == key))
        .collect();
    if found.is_empty() {
        let near: Vec<&str> = entries
            .iter()
            .map(|e| e.schema.as_str())
            .filter(|s| normalise(s).contains(&key) || key.contains(&normalise(s)))
            .take(5)
            .collect();
        if near.is_empty() {
            bail!(
                "the service has no {} called '{name}': `best {} <connection>` lists them",
                kind.singular(),
                kind.path()
            );
        }
        bail!(
            "the service has no {} called '{name}'. Did you mean: {}?",
            kind.singular(),
            near.join(", ")
        );
    }
    match version {
        Some(v) => found.into_iter().find(|e| e.version == v).ok_or_else(|| {
            anyhow::anyhow!(
                "'{bare}' has no version {v}; it has: {}",
                entries
                    .iter()
                    .filter(|e| normalise(&e.schema) == key)
                    .map(|e| e.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }),
        None => Ok(found
            .into_iter()
            .max_by(|a, b| compare_versions(&a.version, &b.version))
            .expect("not empty")),
    }
}

/// Whether an entry matches a listing filter, on its name or description.
pub fn matches(entry: &Entry, filter: Option<&str>) -> bool {
    let Some(f) = filter else { return true };
    let f = f.to_lowercase();
    entry.schema.to_lowercase().contains(&f)
        || entry
            .command_type
            .as_deref()
            .is_some_and(|t| t.to_lowercase().contains(&f))
        || entry
            .description
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains(&f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(schema: &str, version: &str, command_type: Option<&str>) -> Entry {
        Entry::from_value(&json!({"schema": schema, "version": version, "dataschema": format!("https://x/commands/{schema}/{version}"), "commandType": command_type})).unwrap()
    }

    #[test]
    fn finds_by_schema_type_or_glued_name_and_takes_the_latest() {
        let entries = vec![
            entry("deploy-model", "1.0", Some("DeployModel")),
            entry("deploy-model", "2.0", Some("DeployModel")),
            entry("submitannualgrosssalary", "1.0", Some("SubmitAnnualGrossSalary")),
        ];
        assert_eq!(find(&entries, "deploy-model", Kind::Commands).unwrap().version, "2.0");
        assert_eq!(
            find(&entries, "deploy-model/1.0", Kind::Commands).unwrap().version,
            "1.0"
        );
        assert_eq!(find(&entries, "DeployModel", Kind::Commands).unwrap().version, "2.0");
        assert_eq!(
            find(&entries, "submit-annual-gross-salary", Kind::Commands)
                .unwrap()
                .schema,
            "submitannualgrosssalary"
        );
        let err = find(&entries, "deploy-model/9.0", Kind::Commands)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no version 9.0") && err.contains("1.0, 2.0"), "{err}");
        let err = find(&entries, "model", Kind::Commands).unwrap_err().to_string();
        assert!(err.contains("Did you mean: deploy-model"), "{err}");
        assert!(find(&entries, "zzz", Kind::Queries)
            .unwrap_err()
            .to_string()
            .contains("best queries <connection>"));
    }

    #[test]
    fn the_envelope_type_is_stated_or_derived() {
        assert_eq!(entry("deploy-model", "1.0", Some("Deploy")).envelope_type(), "Deploy");
        assert_eq!(entry("deploy-model", "1.0", None).envelope_type(), "DeployModel");
    }

    #[test]
    fn impact_defaults_to_the_strict_level() {
        let e = Entry::from_value(&json!({"schema": "pay", "version": "1.0", "impact": {"categories": ["financial"], "warning": "Moves money"}})).unwrap();
        let impact = e.impact.unwrap();
        assert_eq!(impact.confirmation, "required");
        assert_eq!(impact.warning.as_deref(), Some("Moves money"));
        assert!(Entry::from_value(&json!({"schema": "read", "version": "1.0"}))
            .unwrap()
            .impact
            .is_none());
    }
}
