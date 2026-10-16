# Querya extension template

A working database-driver extension for [Querya Desktop](https://github.com/QueryaHub/Querya-Desktop),
written in Rust, with the dev container, test database and CI already set up.
Click **Use this template**, rename it, and replace the Redis client with the
database you write the driver for.

The driver here speaks the Redis protocol (a few lines of RESP over TCP, no
dependencies besides `serde_json`), so you can read the whole thing in ten
minutes and still see a real connection, a real schema tree and real queries.

## Try it

Open the repository in a dev container (VS Code: *Reopen in Container*, or
`devcontainer up`). It installs Rust, Docker and `querya-ext-tester`, starts the
test database and builds `dist/`. Then:

```bash
scripts/test.sh
```

```
Extension example.redis-template against Querya Desktop 0.5.0
  PASS manifest
  PASS launch
  PASS system.handshake
  ...
  PASS system.shutdown
12 passed, 0 warnings, 0 failed, 0 skipped
```

Without a dev container: install Rust and Docker, take `querya-ext-tester` from
a [Querya Desktop release](https://github.com/QueryaHub/Querya-Desktop/releases)
(0.4.19 or newer), then

```bash
docker compose -f testenv/compose.yml up -d --wait   # DB_PORT=16379 if 6379 is taken
DB_PORT=6379 scripts/test.sh
```

Ask the tester what it sent and got with `--trace trace.json`; see
[Testing extensions in CI](https://github.com/QueryaHub/Querya-Desktop/blob/main/docs/extension-testing.md).

## What is where

| Path | What |
|------|------|
| `manifest.json` | id, entry point (`main`), supported host versions (`engines`), the format of the manifest itself (`manifestVersion`, 1), sandbox permissions, the driver it contributes |
| `assets/connection_form.json` | the connection form Querya Desktop shows |
| `src/main.rs` | the protocol loop: one JSON-RPC message per line on stdin, one answer per line on stdout. Logs go to stderr, never to stdout |
| `src/driver.rs` | **what each `db.*` method does.** Start here |
| `src/resp.rs` | the database client. Replace it with yours |
| `scripts/package.sh` | builds and lays out `dist/`, the folder Querya Desktop and the tester read |
| `testenv/compose.yml` | the test database (image with the seeds already in) |
| `.github/workflows/test.yml` | lint, unit tests, and the tester against each host version in a matrix |

## The methods

| Method | Required | Returns |
|--------|----------|---------|
| `system.handshake`, `system.shutdown` | yes | anything; after `shutdown` the process must exit with code 0 |
| `db.connect`, `db.disconnect` | yes | `{"connected": true, "serverVersion": "..."}` / anything |
| `db.query` | yes | `{"columns": ["a", ...], "rows": [[...], ...]}`. Any other shape shows an empty grid |
| `db.getSchemaTree` | yes | `{"roots": [{"id", "label", "nodeType", "children": [...]}]}` |
| `system.ping`, `db.getCapabilities`, `db.getServerStats`, `db.getTableSchema` | recommended | the host falls back when they fail |

`system.injectCredentials` delivers the password in memory: do not expect it in
`db.connect` from the real app (the tester sends it in `db.connect`, the
template accepts both). Never put a password in argv, the environment or a log.

## Make it yours

1. Change `id`, `name`, `publisher` and `driverId` in `manifest.json`, and the
   package name in `Cargo.toml` (keep `scripts/package.sh` and `main` in sync).
2. Replace `src/resp.rs` with the client of your database and adapt `connect`,
   `query`, `schema_tree` and `table_schema` in `src/driver.rs`.
3. Swap the image in `testenv/compose.yml`:
   `ghcr.io/queryahub/querya-testdb-{postgres,mysql,mongo,clickhouse,redis}`.
4. Set the host versions you support in the CI matrix and in `engines`.

## Sandbox

Querya Desktop runs the driver in an OS sandbox: no filesystem except a scratch
directory, network only to the connection host. `scripts/test.sh` passes
`--sandbox`, so what works here works there. Needs `bubblewrap` on Linux; in a
container the sandbox can be blocked, then the tester says so in its report.
