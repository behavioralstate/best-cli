# How best-cli is built and shipped

> This is the one place the lifecycle of this repository is written down. Open work of any kind (a slice, a bug, a
> decision) is an issue on the Behavioralstate board, never a file: the epic
> [behavioralstate/spec#52](https://github.com/behavioralstate/spec/issues/52) and its sub-issues are the plan.

## 1. Intake

- Work is a slice of the epic, or an issue filed next to it on the same board, with the problem, the outcome from the
  person's side, numbered decisions, and a "Done when" list.
- **The BEST spec is stable** ([behavioralstate/spec#37](https://github.com/behavioralstate/spec/issues/37) holds the
  rule). What a service does that the spec does not mention is that service's own: never an entry on #37, a spec
  issue or a spec pull request. Only the owner changes the spec, on proof that two independent implementations read
  the same sentence differently and a client failed, or that a client could not be written from the text; a session
  that believes it holds that proof brings it to the owner in the chat and stops.
- Ambiguity is resolved by reading the spec, best-mcp's source and the running code, never by guessing.

## 2. Design rules every change is held to

- Everything is driven by the manifest and the catalogues at run time. No code knows one service from another; no
  field, type, path or parameter is written in for a service; nothing is cached across runs. A command's envelope is
  built as best-mcp builds it; a query's answer is printed as the service sent it.
- The credential store is best-mcp's file, entry for entry: same path, same variable, same members, same name rule. A
  login never re-onboards or edits a name that exists; what another program wrote passes through untouched; the file
  is backed up before every write. A credential is never printed, logged or put in an error, and never follows a
  redirect or goes to a host other than the one its sign-in resolved to.
- A refusal is the service's own words: code, message and details are printed, never rephrased or swallowed. Exit
  codes mean something, `--json` is on every command, and nothing is coloured or animated.
- A high-impact command (the catalogue's `impact`) is confirmed per submission; `--yes` is the script's answer; without
  a terminal the command is not sent without it.
- The person approves in their own browser: the link and the code are shown, never opened.
- Instructions, the README and this file state the current rule, not its history; the history is in git.
- The README's "Try it on your machine" is the person's path, kept true by the change that moves it: a flag, a command,
  what the terminal shows, changes that section in the same commit.

## 3. Build

- Tests first where one applies, against the fake surface in `tests/cli.rs`: the sign-in, the store, the catalogues,
  the envelope, the refusals, the events.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; then the binary against a real service:
  best-peer on the fakes (`BEST_PEER_RUNTIME=fake BEST_PEER_GPU=none`), signed in by the device flow and admitted with
  the real `best-peer approve`, and the local DotQuant BaaS when it runs. `best-validate` validates servers, not
  clients, so it does not gate this repository; the services it is run against have passed it.
- The acceptance transcript in `acceptance/` is remade when a change moves what the person sees in it.

## 4. Ship

- **Every change is made on `main`.** No feature branches, no pull requests: one commit per piece of work, the subject
  saying what changed and why, ending with the assistant's attribution line.
- **Before the push, the review gate**: `/best-review`, the owner-run review of everything `main` holds that origin
  does not. Every finding is fixed on `main` with a test or set aside with its reason in the report; nothing is dropped
  silently.
- **The push is the release path.** `main` is what people install from `cargo install --git`; a tag is what the
  install scripts resolve.
- **A release is a tag.** `scripts/release.sh <version> -m "<what it is>"` runs only on a clean `main` level with
  origin, so everything it releases has passed the gate; it bumps the version in `Cargo.toml`, commits
  `Release <version>`, which holds nothing but the version and is the one commit that skips the gate, tags `v<version>`
  with the message and pushes both. The `release` workflow refuses a tag that is not on `main`, not on a
  `Release <version>` commit or not equal to `Cargo.toml`, waits for `ci` to pass on that commit, runs the checks on
  the locked tree, builds the binary for Linux x86_64 and aarch64 (musl, static), Windows x86_64 and macOS Apple
  silicon, checks each says the version, and publishes each archive twice, byte for byte the same: under its versioned
  name, for pinning, and under a name without the version that `releases/latest/download` resolves, so the install
  scripts and the README name no version; with checksums, a build provenance attestation and notes made from the
  release commit's message and the commits since the previous release. Before 1.0 the minor goes up with a capability
  and the patch with a fix. A change to the workflow is tried with its `workflow_dispatch`, which builds and keeps the
  archives as run artifacts and publishes nothing, whatever ref it is started from.

## 5. What we do not do

- No release of a commit the review gate has not passed; no tag made by hand.
- No force-pushes, no push with an unfixed review finding or a red check.
- No plan file, no `TODO`, no dated narrative: open work is an issue.
- No service-specific code, no catalogue cache, no credential in any output.
