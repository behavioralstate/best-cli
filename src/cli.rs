//! The command line as the person types it.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// The command line for every BEST service.
///
/// Sign in once with `best login <name> --manifest <address>`, approving a code in your own browser.
/// The credential is kept where your agent's best-mcp keeps its own, so the two share sign-ins.
/// Everything else is read from the service's manifest and catalogues: nothing here is written for
/// one service.
#[derive(Debug, Parser)]
#[command(name = "best", version, propagate_version = true, max_term_width = 100)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Args)]
pub struct Global {
    /// Print the service's JSON on stdout instead of a table or a line.
    #[arg(long, global = true)]
    pub json: bool,

    /// A PEM file of certificates to trust besides the system's: a service's self-made certificate.
    #[arg(long, global = true, env = "BEST_CA_FILE", value_name = "PEM")]
    pub ca_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in to a service: approve a code in your own browser, keep the credential under NAME.
    Login(LoginArgs),

    /// Forget a connection and its credential (the service's own revocation is its command).
    Logout {
        /// The connection's name.
        name: String,
    },

    /// List the connections you are signed in to.
    Connections,

    /// The commands a connection's service accepts, with their descriptions.
    Commands {
        /// The connection (see `best connections`).
        name: String,
        /// Only entries whose name or description contains this.
        filter: Option<String>,
    },

    /// The queries a connection's service answers, with their descriptions.
    Queries {
        /// The connection (see `best connections`).
        name: String,
        /// Only entries whose name or description contains this.
        filter: Option<String>,
    },

    /// Print the JSON Schema document of a command, a query or an event.
    Schema {
        /// The connection (see `best connections`).
        name: String,
        /// The schema name, optionally with a version: `deploy-model`, `deploy-model/1.0`.
        /// An event needs its version.
        schema: String,
        /// Where to look when a command and a query share a name.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
    },

    /// Run a query and print its answer: a table for a list, JSON otherwise (always JSON with --json).
    Query {
        /// The connection (see `best connections`).
        name: String,
        /// The query's schema name.
        schema: String,
        /// A parameter: `name=value`. Repeatable.
        #[arg(long = "param", short = 'p', value_name = "NAME=VALUE")]
        params: Vec<String>,
    },

    /// Send a command: the envelope is built from the catalogue, the payload is yours.
    Send(SendArgs),

    /// Read the service's event log, or follow its live stream.
    Events(EventsArgs),

    /// The service's published workflows: the index, or one recipe with its steps.
    Workflows {
        /// The connection (see `best connections`).
        name: String,
        /// A workflow id.
        id: Option<String>,
    },

    /// Print a manifest: a connection's, or the one at an address (a bare host means its well-known path).
    Manifest {
        /// A connection name, or a manifest address.
        target: String,
    },
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// What to call this connection: letters, digits, '.', '_' and '-', up to 64, starting with a letter or digit.
    pub name: String,

    /// The manifest address: `https://host/.well-known/best`, or a bare host.
    #[arg(long, value_name = "ADDRESS")]
    pub manifest: String,

    /// What the approver reads when asked to admit you. Default: `best-cli on <this machine>`.
    #[arg(long)]
    pub label: Option<String>,
}

#[derive(Debug, Args)]
pub struct SendArgs {
    /// The connection (see `best connections`).
    pub name: String,

    /// The command: its schema name, optionally with a version (`deploy-model`, `deploy-model/1.0`),
    /// or its type (`DeployModel`).
    pub schema: String,

    /// The payload as JSON, or `-` to read it from stdin. Default: `{}`.
    #[arg(long, conflicts_with = "file")]
    pub data: Option<String>,

    /// The payload read from a JSON file.
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,

    /// Join an existing business process: the envelope's `correlationid`. Omitted, the service uses the command's id.
    #[arg(long, value_name = "ID")]
    pub correlation_id: Option<String>,

    /// The envelope id (a fresh UUID when omitted). The same id sent again is refused as a duplicate.
    #[arg(long)]
    pub id: Option<String>,

    /// The envelope `source`, when the schema's description documents a required value. Default `urn:best-cli`.
    #[arg(long)]
    pub source: Option<String>,

    /// Answer yes to the confirmation a high-impact command asks for. Without a terminal, such a command needs it.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Print the envelope that would be posted, and post nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct EventsArgs {
    /// The connection (see `best connections`).
    pub name: String,

    /// Only events of this type (PascalCase).
    #[arg(long = "type", value_name = "TYPE")]
    pub kind: Option<String>,

    /// Only events of this correlation id (the one `best send` printed).
    #[arg(long, value_name = "ID")]
    pub correlation_id: Option<String>,

    /// Only events from this source.
    #[arg(long)]
    pub source: Option<String>,

    /// From this time on (ISO 8601). History only.
    #[arg(long)]
    pub from: Option<String>,

    /// Up to this time (ISO 8601). History only.
    #[arg(long)]
    pub to: Option<String>,

    /// At most this many events: one page of history, or when to stop following.
    #[arg(long)]
    pub limit: Option<u32>,

    /// The `nextCursor` of the previous page. History only.
    #[arg(long)]
    pub after: Option<String>,

    /// Follow the live stream instead of reading the history: one line per event until Ctrl-C (or --limit).
    #[arg(long, short = 'f')]
    pub follow: bool,

    /// Resume a stream after this event id (sent as Last-Event-ID). With --follow.
    #[arg(long, requires = "follow", value_name = "ID")]
    pub last_event_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    Command,
    Query,
    Event,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }
}
