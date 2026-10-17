# best

`best` is the command line for every [Behavioral State Protocol](https://behavioralstate.io) service: one binary, no
configuration, driven by the service's own manifest and catalogues. You sign in by approving a code in your own
browser, and the credential goes where your agent's `@behavioralstate/best-mcp` keeps its own, so a person and their
agent share sign-ins. Nothing in it is written for one service: what `best` can do on a service is what that service
publishes.

The plan is the epic [behavioralstate/spec#52](https://github.com/behavioralstate/spec/issues/52) on the
[Behavioralstate board](https://github.com/orgs/Amarcode-Studios/projects/4); the board is the plan, no plan file lives
here. How the work is made and shipped is [SDLC.md](SDLC.md).

## Try it on your machine

**1. Install.** One line, on Linux and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/behavioralstate/best-cli/main/install.sh | sh
```

It takes the newest release's archive for your machine, checks it against `SHA256SUMS`, and puts `best` in
`~/.local/bin` (`BEST_INSTALL_DIR` chooses another place). On Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/behavioralstate/best-cli/main/install.ps1 | iex
```

Or take an archive from the [releases](https://github.com/behavioralstate/best-cli/releases) page by hand
(`best-x86_64-unknown-linux-musl.tar.gz`, `best-aarch64-unknown-linux-musl.tar.gz`, `best-aarch64-apple-darwin.tar.gz`,
`best-x86_64-pc-windows-msvc.zip`, each with `SHA256SUMS` and a build provenance attestation:
`gh attestation verify <archive> --repo behavioralstate/best-cli`), or build it with
`cargo install --git https://github.com/behavioralstate/best-cli`. `best --version` says which it is.

**2. Sign in.** Name the connection and give the manifest address, or just the host:

```bash
best login lab-peer --manifest https://peer.example.com:7777/.well-known/best
best login dotquant --manifest baas.dotquant.io
```

```
Resolved baas.dotquant.io to https://baas.dotquant.io/.well-known/best

Open this link in your own browser, sign in and approve:

    https://dotquant.io/activate?user_code=8BB9-CB44

The code is 8BB9-CB44. Waiting for your approval (the code expires in 15 minutes)…

Signed in as 'dotquant' (tenant 67e04942-…) at baas.dotquant.io; the credential is durable until revoked.
The credential is in /home/ada/.best-mcp/credentials.json, where best-mcp finds it too. It is never printed.
  io.dotquant.trading  https://baas.dotquant.io/api/best/tenants/67e04942-…
Next: `best queries dotquant` and `best commands dotquant`.
```

`best` never opens the link itself: approving is yours. A name that is already signed in is left as it is
(`best logout <name>` forgets one). Plain `http://` is accepted only on this machine; `--ca-file <pem>` trusts a
certificate the service's operator sent you (a best-peer makes its own).

**3. Read the service.** Everything comes from its catalogues:

```bash
best connections                       # what you are signed in to
best queries dotquant                  # the queries, with their descriptions
best commands dotquant                 # the commands; IMPACT marks the ones that ask first
best schema dotquant list-brokers      # a command's, a query's or an event's JSON Schema
best workflows dotquant                # the service's published recipes
best query dotquant list-brokers       # a table for a list, JSON otherwise
best query dotquant get-broker-cash --param brokerId=0735cfd1-…
best events dotquant --limit 20        # the event log
best events dotquant --follow --type OrderFilled   # the live stream, one line per event
```

**4. Send a command.** The envelope is built from the catalogue (`type`, `dataschema`), the payload is yours:

```bash
best send lab-peer deploy-model --data '{"model": "qwen2.5:0.5b", "engine": "ollama", "memory_bytes": 1073741824}'
```

```
Accepted DeployModel: id 2a6f…, correlation 2a6f….
Follow it with `best events lab-peer --correlation-id 2a6f… --follow`.
```

A command whose catalogue entry carries the `impact` annotation shows its warning and asks before sending; `--yes`
answers for a script, and without a terminal the command is not sent without it. `--dry-run` prints the envelope and
sends nothing; `--file payload.json` and `--data -` (stdin) carry the payload; `--correlation-id` joins an existing
process.

**5. When something goes wrong,** the answer is the service's own: a BEST refusal prints its code, message and
details (a 400 names the field and the fix); the sign-in endpoints' errors likewise. `--json` on any command prints the
service's JSON on stdout, and errors as JSON on stderr. Exit codes:

| Code | Meaning |
|---|---|
| 0 | done |
| 1 | failed |
| 2 | the command line was wrong |
| 3 | not signed in, the sign-in was refused, or the credential was |
| 4 | a high-impact command was not confirmed |
| 5 | the service refused the request |
| 6 | the service could not be reached, or failed |

## The credential file

`~/.best-mcp/credentials.json` (`BEST_MCP_CREDENTIALS_FILE` moves it, for `best` and best-mcp alike), one entry per
connection name: `tenantId`, `apiKey`, `authType`, `authHeader`, `issuedAt`, `manifest` (the address signed in at) and
`endpoints` (service id → endpoint). `best` reads and writes it entry for entry: what another program wrote, members it
does not know included, comes out as it went in; the file is backed up beside itself (`credentials.json.bak`) before
every write and is readable by its owner only. A connection whose manifest names several services is `name` for the
first and `name/<service id>` for any other.

## Build

```bash
cargo build && cargo test
cargo clippy --all-targets -- -D warnings
```

The tests run the binary against a fake of the BEST surface ([tests/cli.rs](tests/cli.rs)) and need no service. The
run against real ones, best-peer and the DotQuant BaaS, is kept as
[acceptance/best-peer-and-dotquant.md](acceptance/best-peer-and-dotquant.md).
