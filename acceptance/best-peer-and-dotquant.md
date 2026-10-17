# A person drives best-peer and the DotQuant BaaS from the manifest alone

The acceptance run of best-cli ([behavioralstate/spec#52](https://github.com/behavioralstate/spec/issues/52)): the
`best` binary, with nothing but a manifest address, signs in by the device flow, reads the catalogues, runs queries,
sends commands and follows events on two real BEST services, and the credential lands in best-mcp's file. This file is
the record of the run that passed: how it was set up, what it showed, and the terminal output as it came.

## The run

- **The services**: best-peer 0.0.9 built from `57169e4`, on the fakes (`BEST_PEER_RUNTIME=fake BEST_PEER_GPU=none`),
  a fresh data folder, plain http on loopback (`127.0.0.1:7792`), BEST 0.9.17; and the DotQuant BaaS of the local
  test bed at `https://localhost:7175` (BEST 0.9.16, the development certificate the system trusts), through the
  `local` entry that best-mcp 2.5.7 had written in `~/.best-mcp/credentials.json` on 2026-10-02.
- **The client**: `best` 0.1.0, a debug build of the tree this transcript is committed with, run by a script with no
  terminal attached, so every answer below is what a script sees (JSON lines for the stream, no confirmation prompt).
- **Nothing written for it**: the manifest, the catalogues and the schema documents are all it read.
- **The person's part**: the sign-in shows the link and the code; at the peer's machine `best-peer approve <code>`
  admits it, and the login's next poll has the credential, which is never shown.
- **The credentials file** held seven entries best-mcp wrote before the run and holds the same seven after it: the
  peer's entry was added by `login` and removed by `logout`; the others are byte for byte as they were
  (`credentials.json.bak` beside the file is the state before the last write).

## What the run showed

- The device flow end to end on best-peer: the code shown, approved at the machine, the credential stored under the
  name given. The peer's token answer names the door's host as `tenant_id`, which is no workspace: the endpoints are
  the root manifest's, as best-mcp resolves them.
- Both catalogues with their descriptions, a command's schema document, a query answered as a table and as JSON, a
  deploy accepted with the envelope built from the catalogue, its lifecycle read as history then followed live (the
  second deploy's events arrived on the stream, with no history when the follow opened), two refusals in the peer's
  own words (a schema violation naming the fields, a placement refusal naming the room), and the sign-out.
- On the BaaS: the catalogues, a list as a table, a high-impact command stopped before it was sent because no terminal
  could confirm it (exit 4), a dry run, a refusal naming the missing fields (exit 5), an accepted command, the event
  log and the stream.
- Two things the services did that are theirs to look at: the BaaS answered about fifty events for `limit=2` on
  `GET /events`; best-peer answers `tenant_id` as the door's host for a credential at its root.

## The transcript

Each command as typed, its stdout and stderr as shown, and its exit code when it is not 0. The schema documents and
the event lines are complete; one event table is shortened where it ran past a screen.

```
## best-peer (0.0.9 on the fakes, http://127.0.0.1:7792)

$ best login lab-peer --manifest http://127.0.0.1:7792
Resolved http://127.0.0.1:7792 to http://127.0.0.1:7792/.well-known/best

Open this link in your own browser, sign in and approve:

    http://127.0.0.1:7792/auth/verify?user_code=2H9B-L62V

The code is 2H9B-L62V. Waiting for your approval (the code expires in 10 minutes)…

[at the peer's machine]
$ best-peer approve 2H9B-L62V
admitted best-cli on DESKTOP-N6MP7J2 (best-cli) as connection lab-peer with a durable credential that may all, no share set: the agent's next poll gets it

[the login continued:]

Signed in as 'lab-peer' (tenant 127.0.0.1:7792) at 127.0.0.1:7792; the credential is durable until revoked.
The credential is in C:\Users\Riccardo\.best-mcp\credentials.json, where best-mcp finds it too. It is never printed.
  io.best.agents  http://127.0.0.1:7792
Next: `best queries lab-peer` and `best commands lab-peer`.

$ best connections
NAME                        HOST                        TENANT                                SERVICES             SIGNED IN
dev.xml.remundo.com         dev.xml.remundo.com         XML-INT                               io.remundo.eor       2026-09-24
dev.backoffice.remundo.com  dev.backoffice.remundo.com  XML-INT                               io.remundo.eor       2026-09-24
synion.io                   synion.io                   775e6813-a6c8-4a7e-becf-1f4cb1deb406  io.synion.studio     2026-09-24
dotquant.io                 baas.dotquant.io            67e04942-95c8-4e17-9282-83c5f8e95fde  io.dotquant.trading  2026-10-13
local                       localhost:7175              be9e0176-d898-40a0-b558-eaa6f748b6aa  io.dotquant.trading  2026-10-02
staff.remundo.com           staff.remundo.com           staff-wp3zv                           io.remundo.eor       2026-10-08
admin_dotquant              baas.dotquant.io            platform-001                          io.dotquant.admin    2026-10-11
lab-peer                    127.0.0.1:7792              127.0.0.1:7792                        io.best.agents       2026-10-17

$ best queries lab-peer
QUERY               VERSION  DESCRIPTION
get-hardware-state  1.0      The accelerators on this host right now: each device's compute APIs with their versions, …
get-workloads       1.0      What this peer runs for you now: each deployment with its model, engine, device, the memo…
get-workspaces      1.0      Who you are on this door and where you belong: identity, the opaque id of your credential…

$ best commands lab-peer
COMMAND            VERSION  IMPACT  DESCRIPTION
pull-image         1.0              Makes a container image present on this peer, so a later deploy or job can start at once.…
deploy-model       1.0              Serves a model on this peer with the engine named, on a device that offers an API the eng…
undeploy-model     1.0              Stops a deployment and frees its device memory. Refused when no such deployment is live, …
run-job            1.0              Runs an image once, with the arguments given, and tells its end. With memory_bytes the jo…
stop-job           1.0              Stops a running job. Refused when no such job is running, or when another credential star…
create-workspace   1.0              Makes a workspace, the organization or team that holds clouds and admits the identities w…
invite-member      1.0              Invites an identity into a workspace you own, as a member. The identity is the value get-…
accept-invitation  1.0              Accepts an invitation into a workspace, making you a member of it: from then its door ope…
leave-workspace    1.0              Leaves a workspace you are a member of: its door closes to your credential, an event stre…

$ best schema lab-peer deploy-model
{
  "$id": "http://127.0.0.1:7792/commands/deploy-model/1.0",
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "additionalProperties": false,
  "commandType": "DeployModel",
  "completes": [
    "DeploymentEnded",
    "DeploymentFailed",
    "DeploymentReady",
    "OutOfMemory"
  ],
  "description": "Serves a model on this peer with the engine named, on a device that offers an API the engine runs on and has the memory asked for free. Refused at once, with the reason and what to do, when no device fits. Otherwise the facts follow on the stream with this command's correlation id: ImagePulling and ImagePulled, DeploymentStarted, WeightsLoading, then DeploymentReady with the OpenAI-compatible endpoint to send prompts to, or DeploymentFailed, which complete it. Later, OutOfMemory or DeploymentEnded if the engine ends it or stops on its own, under the same correlation id. Needs the deploy scope, and room in the credential's share when one was set.",
  "produces": [
    "ImagePulling",
    "ImagePulled",
    "DeploymentStarted",
    "WeightsLoading",
    "DeploymentReady",
    "DeploymentFailed",
    "OutOfMemory",
    "DeploymentEnded"
  ],
  "properties": {
    "device": {
      "description": "A device id from get-hardware-state, to pin the placement; otherwise the fitting device with the most room",
      "type": "string"
    },
    "engine": {
      "description": "ollama runs on cuda, rocm or cpu; vllm on cuda",
      "enum": [
        "ollama",
        "vllm"
      ],
      "type": "string"
    },
    "memory_bytes": {
      "description": "The device memory the model needs; the deploy is placed where that much is free after what is already promised",
      "minimum": 1,
      "type": "integer"
    },
    "model": {
      "description": "The model as the engine names it: the name in the engine's own library or registry, tag included where the engine uses one",
      "minLength": 1,
      "type": "string"
    }
  },
  "required": [
    "model",
    "engine",
    "memory_bytes"
  ],
  "title": "DeployModel",
  "type": "object"
}

$ best query lab-peer get-hardware-state
compute          id     memory                                               name                                     power_w  temperature_c  utilisation_pct  vendor
[{"api":"cpu"}]  cpu-0  {"free_bytes":8833220608,"total_bytes":30886772736}  AMD Ryzen 9 6900HX with Radeon Graphics                          11               {"other":"host"}
driver: 
observed_at: 2026-10-17T15:26:36.0631098Z

$ best query lab-peer get-workloads
{
  "deployments": [],
  "held_bytes": 0,
  "jobs": [],
  "share_bytes": null
}

$ best send lab-peer deploy-model --id 33333333-4444-4555-8666-777777777777 --data {"model":"qwen2.5:0.5b","engine":"ollama","memory_bytes":1073741824}
Accepted DeployModel: id 33333333-4444-4555-8666-777777777777, correlation 33333333-4444-4555-8666-777777777777.
Follow it with `best events lab-peer --correlation-id 33333333-4444-4555-8666-777777777777 --follow`.

$ best events lab-peer --correlation-id 33333333-4444-4555-8666-777777777777 --follow --limit 6
{"correlationid":"33333333-4444-4555-8666-777777777777","data":{"image":"ollama/ollama","progress_pct":50},"datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/image-pulling/1.0","id":"f24598cf-59cf-4d9c-b651-d59d1e530fec","source":"io.best.agents","specversion":"1.0","time":"2026-10-17T15:26:36.1527697Z","type":"ImagePulling"}
{"correlationid":"33333333-4444-4555-8666-777777777777","data":{"image":"ollama/ollama","progress_pct":100},"datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/image-pulling/1.0","id":"b079b1c2-0682-4d40-bb6b-8a6ee0ab3363","source":"io.best.agents","specversion":"1.0","time":"2026-10-17T15:26:36.1528339Z","type":"ImagePulling"}
{"correlationid":"33333333-4444-4555-8666-777777777777","data":{"image":"ollama/ollama"},"datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/image-pulled/1.0","id":"fb7b08e5-2ecd-4b42-9129-5eb0eab0debd","source":"io.best.agents","specversion":"1.0","time":"2026-10-17T15:26:36.1551588Z","type":"ImagePulled"}
{"correlationid":"33333333-4444-4555-8666-777777777777","data":{"container":"fake-1","deployment":"dep-763e067e7140","device":"cpu-0","engine":"ollama","holder":"37908385ff51bf7a","memory_bytes":1073741824,"model":"qwen2.5:0.5b","port":41434},"datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/deployment-started/1.0","id":"33290d1f-b53f-406d-84b7-27558e1872e6","source":"io.best.agents","specversion":"1.0","time":"2026-10-17T15:26:36.1551863Z","type":"DeploymentStarted"}
{"correlationid":"33333333-4444-4555-8666-777777777777","data":{"deployment":"dep-763e067e7140","model":"qwen2.5:0.5b","progress_pct":25},"datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/weights-loading/1.0","id":"d60cff3b-e7e9-4930-b50b-356dc836dcdb","source":"io.best.agents","specversion":"1.0","time":"2026-10-17T15:26:36.1553307Z","type":"WeightsLoading"}
{"correlationid":"33333333-4444-4555-8666-777777777777","data":{"deployment":"dep-763e067e7140","model":"qwen2.5:0.5b","progress_pct":100},"datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/weights-loading/1.0","id":"72c12d85-2a2d-4ab6-b05a-e86656532c2e","source":"io.best.agents","specversion":"1.0","time":"2026-10-17T15:26:36.1553439Z","type":"WeightsLoading"}

$ best query lab-peer get-workloads
deployment        device  endpoint                                               engine  holder            memory_bytes  model         peer  phase
dep-763e067e7140  cpu-0   http://127.0.0.1:7792/deployments/dep-763e067e7140/v1  ollama  37908385ff51bf7a  1073741824    qwen2.5:0.5b        ready
held_bytes: 1073741824
jobs: []
share_bytes: 

$ best events lab-peer --follow --correlation-id 44444444-5555-4666-8777-888888888888 --limit 3 &    # nothing in the history yet: the stream delivers

$ best send lab-peer deploy-model --id 44444444-5555-4666-8777-888888888888 --data {"model":"qwen2.5:1.5b","engine":"ollama","memory_bytes":2147483648}
Accepted DeployModel: id 44444444-5555-4666-8777-888888888888, correlation 44444444-5555-4666-8777-888888888888.
Follow it with `best events lab-peer --correlation-id 44444444-5555-4666-8777-888888888888 --follow`.
[the follow printed, then exited at its limit:]
{"specversion":"1.0","id":"a5e2a415-82d1-46b7-91a5-f47a8b1212eb","correlationid":"44444444-5555-4666-8777-888888888888","source":"io.best.agents","type":"ImagePulling","datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/image-pulling/1.0","time":"2026-10-17T15:26:38.3386035Z","data":{"image":"ollama/ollama","progress_pct":50}}
{"specversion":"1.0","id":"a23e9ef3-1b43-43d7-bc6f-578d0f1bca88","correlationid":"44444444-5555-4666-8777-888888888888","source":"io.best.agents","type":"ImagePulling","datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/image-pulling/1.0","time":"2026-10-17T15:26:38.3386215Z","data":{"image":"ollama/ollama","progress_pct":100}}
{"specversion":"1.0","id":"c24335dd-cb95-40f8-bbf5-5c12882cca1a","correlationid":"44444444-5555-4666-8777-888888888888","source":"io.best.agents","type":"ImagePulled","datacontenttype":"application/json","dataschema":"http://127.0.0.1:7792/events/image-pulled/1.0","time":"2026-10-17T15:26:38.3386314Z","data":{"image":"ollama/ollama"}}

$ best send lab-peer deploy-model --data {"model":"qwen2.5:0.5b"}
error: SCHEMA_VALIDATION: data: "engine" is a required property. Read the schema at the catalogue's dataschema for deploy-model and send what it asks.
details: {"errors":[{"message":"\"engine\" is a required property","path":"data"},{"message":"\"memory_bytes\" is a required property","path":"data"}]} (HTTP 400)
[exit 5]

$ best send lab-peer deploy-model --data {"model":"qwen2.5:0.5b","engine":"ollama","memory_bytes":99999999999999}
error: INSUFFICIENT_MEMORY: no device has 99999999999999 bytes for qwen2.5:0.5b on ollama: cpu-0 (AMD Ryzen 9 6900HX with Radeon Graphics) has 8831856640 free of 30886772736 with 3221225472 promised. Undeploy a model (UndeployModel) or stop a job (StopJob) named in the details, or ask for less memory_bytes
details: {"devices":[{"free_bytes":8831856640,"holders":[{"deployment":"dep-763e067e7140","memory_bytes":1073741824,"model":"qwen2.5:0.5b"},{"deployment":"dep-ea0a99393f7d","memory_bytes":2147483648,"model":"qwen2.5:1.5b"}],"id":"cpu-0","name":"AMD Ryzen 9 6900HX with Radeon Graphics","promised_bytes":3221225472,"room_bytes":8831856640,"total_bytes":30886772736}],"memory_bytes":99999999999999} (HTTP 400)
[exit 5]

$ best workflows lab-peer
io.best.agents publishes no workflows.

$ best query lab-peer get-workspaces
{
  "identity": "37908385ff51bf7a",
  "invitations": [],
  "workspaces": []
}

$ best logout lab-peer
Forgot 'lab-peer'. The credential itself stays valid until the service revokes it (its catalogue says how).

## DotQuant BaaS (local test bed, https://localhost:7175, the 'local' entry best-mcp wrote)

$ best connections
NAME                        HOST                        TENANT                                SERVICES             SIGNED IN
dev.xml.remundo.com         dev.xml.remundo.com         XML-INT                               io.remundo.eor       2026-09-24
dev.backoffice.remundo.com  dev.backoffice.remundo.com  XML-INT                               io.remundo.eor       2026-09-24
synion.io                   synion.io                   775e6813-a6c8-4a7e-becf-1f4cb1deb406  io.synion.studio     2026-09-24
dotquant.io                 baas.dotquant.io            67e04942-95c8-4e17-9282-83c5f8e95fde  io.dotquant.trading  2026-10-13
local                       localhost:7175              be9e0176-d898-40a0-b558-eaa6f748b6aa  io.dotquant.trading  2026-10-02
staff.remundo.com           staff.remundo.com           staff-wp3zv                           io.remundo.eor       2026-10-08
admin_dotquant              baas.dotquant.io            platform-001                          io.dotquant.admin    2026-10-11

$ best queries local
QUERY                           VERSION  DESCRIPTION
list-brokers                    1.0      List all configured broker accounts for this tenant, with each connection's execution fac…
get-broker-order                1.0      Read the current state of an order directly from the broker, for brokers that expose a tr…
get-broker-position             1.0      Read how much of a symbol a broker connection holds, so a sell can be sized against a rea…
get-broker-cash                 1.0      Read a broker connection's free (spendable) cash, so a buy can be sanity-checked against …
list-orders                     1.0      List this tenant's own orders (newest first) from the platform's records: status, fill pr…
estimate-order-cost             1.0      Estimate the broker cost of ONE order before suggesting it: commission, FX fee and UK sta…
symbol-search                   1.0      Search for tradeable symbols by name or ticker. Use this before subscribe-price-feed to r…
list-price-feeds                1.0      List all configured price feeds for this tenant.
get-market-state                1.0      Get current market open/closed state for an exchange or specific symbol. Returns cached s…
search-index-symbols            1.0      Search the price data provider's index catalog for valid index symbols and exchanges. Use…
get-exchange-session            1.0      Returns the open/close UTC times and IANA timezone for a given exchange on a specific dat…
list-webhooks                   1.0      List this tenant's webhook endpoints: each one's id (a route's WebhookId), the url it pos…
list-watchlist-panels           1.0      The tenant's watchlist panel organisation: named panels and the symbols explicitly assign…
list-portfolios                 1.0      List this tenant's portfolios — the swing/long-term lane's intent: holdings, target weigh…
list-portfolio-proposals        1.0      List contribution proposals — each a fully resolved plan (per-holding orders and banked s…
list-insider-mandates           1.0      This tenant's insider-following mandates: intent (budget, one connection, policy), whethe…
list-insider-mandate-actions    1.0      A mandate's ledger, newest first: every entry and exit the daily round dispatched, skippe…
list-insider-mandate-notes      1.0      The notes on this tenant's insider mandates, newest first — named metadata written with a…
list-history                    1.0      List this tenant's own history, newest first. Two things are in it. (1) INSTRUCTIONS: por…
list-portfolio-notes            1.0      List the notes on this tenant's portfolios, newest first — named metadata written with an…
list-portfolio-recommendations  1.0      List composition recommendations on this tenant's portfolios, newest first — a review's s…
portfolio-performance           1.0      Per-holding value, P/L and return% plus portfolio totals, computed from the platform's OW…
portfolio-performance-history   1.0      A daily rate-of-return series computed from the platform's OWN fill records: the cost bas…
attribution-summary-history     1.0      The daily rate-of-return series behind the all-activity headline: EVERY own fill on every…
attribution-by-instrument       1.0      Trade attribution grouped by instrument: for each (broker connection, feed) with filled o…
attribution-by-origin           1.0      Trade attribution grouped by ORIGIN: one group per (originating lane, broker connection) …
attribution-summary             1.0      Account-wide attribution summary in ONE requested display currency: every own fill on eve…
list-theses                     1.0      List DSL theses (screener/strategy specs) visible to the tenant: their own plus the platf…
list-algorithms                 1.0      List the strategy algorithms configured for this tenant, with their running state. An alg…
list-signal-routes              1.0      List this tenant's signal routes — where each SOURCE's events flow: a strategy's signals …
list-algorithm-signals          1.0      List the decisions this tenant's strategy algorithms have made — one entry per position c…
run-backtest                    1.0      Run a backtest of a compiled strategy against a price feed's stored bars and return the r…
get-tenant-profile              1.0      Who am I? The authenticated tenant's identity and account status: tenant id, account emai…
get-support-thread              1.0      This account's conversation with the people who run dotQuant: what was sent with submit-f…
get-instrument-type             1.0      Resolve a symbol's instrument type (e.g. 'Common Stock', 'ETF', 'Index') via symbol searc…
get-company-profile             1.0      What the company (or fund) behind a symbol IS: business description, sector/industry/coun…
get-fundamentals                1.0      Get financial fundamentals for a symbol feed. Used by analysis agents. Returns income sta…
list-insider-transactions       1.0      Insider transactions for a symbol feed, newest filing first, with a summary of open-marke…
get-agent-catalogue             1.0      List the AI analysis agent personas — name, investment style, tagline, objective, and gui…
get-market-regime               1.0      The market-regime FACT per region (US / Europe / Asia): a deterministic, breadth-averaged…
get-analysis                    1.0      Get AI agent analysis results for a symbol feed. Returns signals, scores, and reasoning f…
get-analysis-metrics            1.0      Get the objective 'Key Metrics' appraisal for a symbol feed — persona-independent derived…
get-market-news                 1.0      Central-bank and government releases — the Federal Reserve Board, the European Central Ba…
get-macro-indicators            1.0      Macro indicators per region (US / Europe / Asia): inflation, GDP growth, the policy rate,…
get-price-variations            1.0      Percentage price variations for a symbol feed over standard windows (1 day, 1 week, 1 mon…
get-news-sentiment              1.0      Aggregated news sentiment for a symbol feed over a given period: article counts by sentim…
get-symbol-situation            1.0      The current situation of one of your subscribed symbol feeds: most recent stored price wi…

$ best query local list-brokers
id                                    name  displayName    accountMode  configured  connected  execution                                                     observedFees
0735cfd1-8ac2-42a3-b672-361ffcdcd5a4  IBKR  Paper Trading  paper        true        false      {"fractionalShares":false,"quantityDecimals":0,"minOrderVal…  {"orders":3,"commissionTotal":7.13,"commissionCurrency":"GB…
6d753310-6f7e-49a9-9413-2f1fee06d0e1  IBKR  Live Trading   live         true        false      {"fractionalShares":false,"quantityDecimals":0,"minOrderVal…  {"orders":1,"commissionTotal":3,"commissionCurrency":"EUR",…
31d7bd6e-d97c-45b7-b94c-cd74b5b1d0c0  T212  Paper Trading  paper        true        true       {"fractionalShares":true,"quantityDecimals":3,"minOrderValu…  {"orders":6,"commissionTotal":1.16,"commissionCurrency":"GB…
987141a5-9845-4bde-8b3a-2af82b202da2  T212  Live Trading   live         true        true       {"fractionalShares":true,"quantityDecimals":3,"minOrderValu…  {"orders":1,"commissionTotal":0.1,"commissionCurrency":"GBP…

$ best query local get-market-state --param exchange=NASDAQ
{
  "exchange": "NASDAQ",
  "ticker": null,
  "isMarketOpen": false,
  "marketClosedAt": "2026-10-17T15:26:39.5765156Z",
  "marketNextOpenUtc": "2026-10-19T13:30:00Z",
  "marketLastClosePrice": null,
  "source": "live"
}

$ best commands local price
COMMAND                    VERSION  IMPACT  DESCRIPTION
analyze-symbol             1.0              Triggers AI investment-agent analysis for a symbol (Warren Buffett, Charlie Munger, etc.)…
appraise-symbol            1.0              Triggers an objective, persona-independent fundamental appraisal of a symbol — the non-ag…
backfill-price-history     1.0              Ask the platform to store MORE history for one of your subscribed feeds — for example thr…
configure-algorithm        1.0              GUIDED SEQUENCE — the run-a-strategy-live workflow (GET /workflows/io.dotquant.workflows.…
configure-insider-mandate  1.0              GUIDED SEQUENCE — read the recipe first: design-an-insider-mandate walks the person throu…
follow-symbol              1.0              GUIDED SEQUENCE — the add-symbol-to-watchlist workflow (GET /workflows/io.dotquant.workfl…
request-portfolio-review   1.0              GUIDED SEQUENCE — step of the manage-a-portfolio workflow (GET /workflows/io.dotquant.wor…
subscribe-price-feed       1.0              GUIDED SEQUENCE — the add-symbol-to-watchlist workflow (GET /workflows/io.dotquant.workfl…
unsubscribe-price-feed     1.0              Deactivates the platform price feed for a symbol: the platform stops collecting new bars …

$ best schema local backfill-price-history
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "BackfillPriceHistory",
  "description": "Ask the platform to store MORE history for one of your subscribed feeds — for example three months of intraday bars before an intraday backtest (part of the backtest-a-strategy recipe). The feed is the instrument: the platform fills in ticker, exchange and, unless you name one, the feed's stored intraday interval (1min/5min/1h; 3min/15min/30min/2h/4h are folded from those at read time and are never stored). Bars land in the read model a minute or so later (long windows are fetched in chunks of ~100 days, so a year of 1min takes a few minutes): read get-price-history-coverage to watch firstBar move back to your From, then run-backtest. The current session is never available until ~3 hours after the close. Produces no event and no result — re-send if coverage does not move. A feed you have not subscribed does nothing (subscribe-price-feed first).",
  "type": "object",
  "properties": {
    "CorrelationId": {
      "type": "string",
      "minLength": 3,
      "maxLength": 64,
      "pattern": "^[A-Z0-9][A-Z0-9./-]*-[A-Z0-9][A-Z0-9./-]*$",
      "description": "The feed id, EXCHANGE-TICKER uppercase (e.g. LSE-RR, NASDAQ-MSFT) — see list-price-feeds."
    },
    "TenantId": {
      "type": "string",
      "minLength": 1,
      "maxLength": 64,
      "description": "Your tenant id (must match the route)."
    },
    "From": {
      "type": "string",
      "minLength": 10,
      "maxLength": 40,
      "description": "Start of the history you want stored, ISO 8601 date or date-time (UTC), e.g. 2026-06-01 or 2026-06-01T00:00:00Z. Older than ~2 years of 1min is rarely served by the provider."
    },
    "To": {
      "type": "string",
      "minLength": 10,
      "maxLength": 40,
      "description": "End of the range, ISO 8601. Omit for now."
    },
    "Interval": {
      "type": "string",
      "enum": [
        "1min",
        "5min",
        "1h"
      ],
      "description": "The stored series to extend. Omit for the feed's own resolved interval (what get-price-history-coverage reports as storedInterval)."
    }
  },
  "required": [
    "CorrelationId",
    "TenantId",
    "From"
  ],
  "additionalProperties": false,
  "commandType": "BackfillPriceHistory",
  "workflows": [
    "io.dotquant.workflows.backtest-a-strategy"
  ]
}

$ best send local close-account
close-account is a high-impact command (destructive, irreversible; confirmation required).
Warning: The account is closed and its key stops working. This cannot be undone. Say so in those words and get the person's explicit agreement first.
error: Send CloseAccount to io.dotquant.trading (local)? — not asked: there is no terminal. Pass --yes to send it.
[exit 4]

$ best send local backfill-price-history --dry-run --data {"CorrelationId":"NASDAQ-MSFT"}
{
  "post": "https://localhost:7175/api/best/tenants/be9e0176-d898-40a0-b558-eaa6f748b6aa/commands",
  "body": {
    "specversion": "1.0",
    "id": "370eef40-cd18-4005-a84d-b55159f95bc7",
    "source": "urn:best-cli",
    "type": "BackfillPriceHistory",
    "datacontenttype": "application/json",
    "dataschema": "https://localhost:7175/api/best/tenants/be9e0176-d898-40a0-b558-eaa6f748b6aa/commands/backfill-price-history/1.0",
    "time": "2026-10-17T15:26:40.169Z",
    "data": {
      "CorrelationId": "NASDAQ-MSFT"
    }
  }
}
Dry run: nothing was sent.

$ best send local backfill-price-history --data {"CorrelationId":"NASDAQ-MSFT"}
error: INVALID_PAYLOAD:  required: Required properties ["TenantId","From"] are not present
 (HTTP 400)
[exit 5]

$ best send local backfill-price-history --data {"CorrelationId":"NASDAQ-MSFT","TenantId":"be9e0176-d898-40a0-b558-eaa6f748b6aa","From":"2026-10-01"}
Accepted BackfillPriceHistory: id NASDAQ-MSFT, correlation NASDAQ-MSFT.
Follow it with `best events local --correlation-id NASDAQ-MSFT --follow`.

$ best events local --correlation-id NASDAQ-MSFT --limit 2
TIME                      TYPE         CORRELATION  ID                                             DATA
2026-10-17T15:26:40.796Z  PriceTickV1  NASDAQ-MSFT  NASDAQ-MSFT:1min:2026-10-16T23:59:00.0000000Z  {"CorrelationId":"NASDAQ-MSFT","TenantId":"be9e0176-d898-40…
2026-10-17T15:26:40.796Z  PriceTickV1  NASDAQ-MSFT  NASDAQ-MSFT:1min:2026-10-16T23:58:00.0000000Z  {"CorrelationId":"NASDAQ-MSFT","TenantId":"be9e0176-d898-40…
2026-10-17T15:26:40.796Z  PriceTickV1  NASDAQ-MSFT  NASDAQ-MSFT:1min:2026-10-16T23:57:00.0000000Z  {"CorrelationId":"NASDAQ-MSFT","TenantId":"be9e0176-d898-40…
[… 47 more rows: the service answered 50 events for limit=2]

$ timeout 30 best events local --follow --limit 3
{"specversion":"1.0","id":"NASDAQ-MSFT:1min:2026-10-16T23:59:00.0000000Z","source":"io.dotquant.trading","type":"PriceTickV1","datacontenttype":"application/json","dataschema":"https://localhost:7175/api/best/tenants/be9e0176-d898-40a0-b558-eaa6f748b6aa/events/price-tick/1.0","time":"2026-10-17T15:26:41.037Z","correlationid":"NASDAQ-MSFT","data":{"CorrelationId":"NASDAQ-MSFT","TenantId":"be9e0176-d898-40a0-b558-eaa6f748b6aa","Source":"platform","Interval":"1min","Currency":"USD","Open":527.4,"High":527.44,"Low":527,"Close":527.0258,"Volume":28,"Price":527.0258,"Timestamp":"2026-10-16T23:59:00Z","IsDelayed":false}}
{"specversion":"1.0","id":"NYSE-DOCN:1min:2026-10-16T20:05:00.0000000Z","source":"io.dotquant.trading","type":"PriceTickV1","datacontenttype":"application/json","dataschema":"https://localhost:7175/api/best/tenants/be9e0176-d898-40a0-b558-eaa6f748b6aa/events/price-tick/1.0","time":"2026-10-17T15:26:40.987Z","correlationid":"NYSE-DOCN","data":{"CorrelationId":"NYSE-DOCN","TenantId":"be9e0176-d898-40a0-b558-eaa6f748b6aa","Source":"platform","Interval":"1min","Currency":"USD","Open":97.285,"High":98.66,"Low":97,"Close":98.66,"Volume":69,"Price":98.66,"Timestamp":"2026-10-16T20:05:00Z","IsDelayed":false}}
{"specversion":"1.0","id":"NASDAQ-ALAB:1min:2026-10-16T20:05:00.0000000Z","source":"io.dotquant.trading","type":"PriceTickV1","datacontenttype":"application/json","dataschema":"https://localhost:7175/api/best/tenants/be9e0176-d898-40a0-b558-eaa6f748b6aa/events/price-tick/1.0","time":"2026-10-17T15:26:40.994Z","correlationid":"NASDAQ-ALAB","data":{"CorrelationId":"NASDAQ-ALAB","TenantId":"be9e0176-d898-40a0-b558-eaa6f748b6aa","Source":"platform","Interval":"1min","Currency":"USD","Open":285.73,"High":285.73,"Low":285.66,"Close":285.69,"Volume":11,"Price":285.69,"Timestamp":"2026-10-16T20:05:00Z","IsDelayed":false}}
[exit 0]

$ best workflows local
ID                                                        NAME                                                                         DESCRIPTION
io.dotquant.workflows.add-symbol-to-watchlist             Add a symbol to the watchlist                                                Follow a new instrument: resolve it with symbol-search, then send TWO commands — subscrib…
io.dotquant.workflows.research-a-symbol                   Research a symbol                                                            A full read-out on one of the tenant's subscribed symbols: current situation, price trend…
io.dotquant.workflows.organize-watchlist-panels           Organize watchlist panels                                                    Create, rename and populate the tenant's watchlist panels. Panels are pure organisation —…
io.dotquant.workflows.create-a-strategy                   Create or edit a strategy                                                    Turn a plain-English trading idea into a compiled, testable strategy. The compiler classi…
io.dotquant.workflows.backtest-a-strategy                 Backtest a strategy                                                          Test a compiled strategy against a price feed's real stored bars — same engine as the UI'…
io.dotquant.workflows.run-a-strategy-live                 Run a strategy live on a feed                                                Apply a compiled strategy to a price feed and start it — what the chart legend's play but…
io.dotquant.workflows.route-strategy-signals              Send a strategy's signals to a broker, a webhook or your email               Route a strategy's buy/sell signals somewhere, for example 'email me when my strategy sig…
io.dotquant.workflows.route-mandate-and-portfolio-events  Get notified about a mandate or a portfolio, by email or webhook             Route an insider MANDATE's or a PORTFOLIO's events to the user's email or webhook — for e…
io.dotquant.workflows.handle-a-failed-order               Tell the person about an automated order that failed, and what happens next  A strategy route, an insider mandate and a portfolio place orders on their own — nobody i…
io.dotquant.workflows.design-a-portfolio                  Design a long-term portfolio from a mandate                                  Turn a user's investment mandate (e.g. 'beat a global index fund over years, adding money…
io.dotquant.workflows.manage-a-portfolio                  Set up and run a long-term portfolio                                         Create a target-weight portfolio (the swing/long-term lane), enable its monthly review, a…
io.dotquant.workflows.design-an-insider-mandate           Design an insider-following mandate with the person, step by step            The AGENT-FIRST way to create an insider mandate: a guided conversation that turns what t…
io.dotquant.workflows.run-an-insider-mandate              Set up and arm an insider-following mandate                                  Create an INSIDER MANDATE — the autonomous sibling of a portfolio: the user approves it O…
io.dotquant.workflows.review-a-portfolio                  Review one portfolio against its own mandate                                 A REVIEW of one portfolio by an agent: read what the user asked for (their prompt and goa…
io.dotquant.workflows.review-portfolios                   Review the portfolios on one connection together                             A cross-portfolio review by an agent: all the user's portfolios on ONE broker connection,…
io.dotquant.workflows.place-manual-order                  Place a manual order                                                         Place a buy or sell order through one of the tenant's configured broker connections and f…

$ best workflows local io.dotquant.workflows.backtest-a-strategy
Backtest a strategy

Test a compiled strategy against a price feed's real stored bars — same engine as the UI's 'Backtest this' button, with honest fills, per-side costs, drawdown and a buy-and-hold benchmark. Synchronous: the run-backtest QUERY returns the result in seconds (do NOT use the legacy run-backtest command). Money values come back in the feed's currency; winRate is a fraction (0.6 = 60%) while returnPct is a percent (-4.57 = -4.57%).

1. query list-theses/1.0
   Find the strategy to test — it must be kind 'strategy' with a compiled spec. Note its thesisId.
2. query list-price-feeds/1.0 (optional)
   Pick the feed to test on (EXCHANGE-TICKER id). The feed must be subscribed — backtests run on the platform's stored bars.
3. query get-price-history-coverage/1.0
   Check how much stored history the feed has at each interval BEFORE you run: firstBar/lastBar/sessions per stored series, and which stored series a 15min/30min/4h request folds from. If firstBar is later than your window's start, backfill first — a run on three sessions is not a backtest.
4. command backfill-price-history/1.0 (optional)
   Ask for the missing range: CorrelationId + From (To defaults to now, Interval to the feed's storedInterval). Then re-read get-price-history-coverage every ~20 s until firstBar reaches your From — a few minutes for a year of 1min bars. The current session is never available until ~3 hours after the close.
5. query run-backtest/1.0
   Run it (thesisId + feedId; window defaults to the last 2 years of daily bars). Make the window wide enough to warm the strategy's slowest indicator — a 200-day average needs ~a year of bars before its first trade. A spec carrying its own timeframe overrides the interval parameter; the response reports what actually ran. Interpret against the benchmark: totalPnl (net of totalCosts) vs buyAndHoldPnl, plus maxDrawdown and endedInPosition.
6. command compile-thesis/1.0 (optional)
   Iterate: revise the prose, re-send with the same CorrelationId, wait for compiledAt to advance in list-theses (or status 'failed' with an error — one timeframe per strategy; a higher-timeframe filter is not supported), and backtest again.
```
