# best-cli

The command line for every BEST service, as one binary named `best`. The protocol it speaks is
[behavioralstate/spec](https://github.com/behavioralstate/spec) (`SPEC.md`); the reference consumer it mirrors is
`@behavioralstate/best-mcp` in that repository's `mcp-server/`. Read both before changing how a request is made or a
credential is kept. How work is made and shipped is [SDLC.md](../../SDLC.md); open work is on the Behavioralstate board.

## Layout

| Path | What it owns |
|---|---|
| `src/cli.rs` | The command line as the person types it (clap). Every flag's help is a sentence a person reads. |
| `src/app.rs` | The commands on top of the modules: login, logout, connections, commands, queries, schema, query, send, events, workflows, manifest. Nothing here knows one service from another. |
| `src/credentials.rs` | best-mcp's credentials file, read and written entry for entry. The credential is a `Secret` that leaves the module only as a request header. |
| `src/manifest.rs` | An address to a manifest, the registration block, the endpoints a credential opens (tenant manifest when the host is multi-tenant). |
| `src/signin.rs` | The device flow (RFC 8628). The device code never leaves it. |
| `src/catalogue.rs` | The catalogues and how a name on the command line finds an entry. |
| `src/http.rs` | Requests with the credential, no redirects, the error classification the exit codes rest on. |
| `src/output.rs` | stdout for answers, stderr for what is said to the person; tables, JSON, confirmation. |
| `tests/cli.rs` | The binary against a fake of the BEST surface. |
| `acceptance/` | The run against real services, as a transcript. |

## Rules

- **The manifest and the catalogues decide.** A new capability is read from what the service publishes; it is never a
  field, a path or a parameter written in for a service. If a service needs something the protocol does not say, that
  is a spec follow-up, not a special case here.
- **The store is best-mcp's.** Same file, same variable (`BEST_MCP_CREDENTIALS_FILE`), same members in the same order,
  same name rule. An entry that exists is not signed in again or edited by a login; unknown members and other entries
  pass through; the file is backed up before every write and is owner-readable only.
- **No credential in any output.** Not in a message, a table, an error, a log, a test assertion's failure text.
  `Secret` has a redacted `Debug`; keep it that way and keep the type.
- **A refusal is the service's words.** The error body's code, message and details are printed as they came. An exit
  code is chosen from the status, never from the text.
- **stdout is the service's answer, stderr is for the person.** `--json` prints the service's JSON unchanged on
  stdout and errors as JSON on stderr. No colour, no spinner, no animation.
- **A high-impact command asks.** `impact` on the catalogue entry means a warning and a per-submission confirmation;
  `--yes` answers for a script; no terminal and no `--yes` means not sent. An absent or unknown `confirmation` level is
  the strict one.
- **The person approves in their own browser.** The link and the code are shown; nothing opens them.
- **A credential goes only to the host its sign-in resolved to**: no redirects on credentialed requests, endpoints on
  another host are dropped at sign-in, and the tenant manifest must be on the same host.
- **Tests run without a service.** The fake in `tests/cli.rs` serves the surface; a change to what the binary says is a
  change to a test's assertion in the same commit. The acceptance transcript is remade when the person's path moves.
- **Documents state rules, never history.** No dated entries, no "done" lists, no issue pointers as the home of a rule.
