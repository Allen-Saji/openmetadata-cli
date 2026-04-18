# omd — OpenMetadata CLI

Command-line tool for [OpenMetadata](https://github.com/open-metadata/OpenMetadata). Written in Rust.

Dynamic command surface generated from OpenMetadata's OpenAPI spec, plus hand-tuned "smart" commands for the common workflows (search, describe, lineage, quality, CSV import/export). Structured JSON output for scripts and agents. Ships an MCP server mode (`omd mcp`) so AI agents can drive OpenMetadata directly.

Status: **v0.4 (early development)**. Not on crates.io yet.

## Install (source, for now)

```bash
git clone https://github.com/Allen-Saji/openmetadata-cli.git
cd openmetadata-cli
cargo install --path .
```

## Quick start

```bash
omd configure                              # interactive setup
omd auth login                             # paste JWT (input hidden)
omd auth status                            # verify
omd search tables customer                 # find entities
omd describe service.db.schema.table       # show entity details
omd raw GET v1/tables -q limit=5           # escape hatch
omd sync                                   # refresh cached OpenAPI spec
```

## Dynamic commands (v0.2)

After `omd sync`, every tag in the OpenAPI spec becomes a subcommand group and every
operation becomes an action. The shape matches `gh <group> <action>` / `kubectl <group> <action>`.

```bash
omd tables list --limit 10
omd tables get-by-id <id>
omd tables get-by-fqn <fqn> --fields columns,owners
omd tables patch <id> --body @patch.json
omd domains list
omd classifications create --body '{"name":"pii"}'
```

Available action groups surface in `omd --help`; run `omd <group> --help` to see actions,
and `omd <group> <action> --help` for per-operation flags.

Body input forms for any action that accepts a request body:

- inline JSON: `--body '{"name":"orders"}'`
- file: `--body @payload.json`
- stdin: `--body -`

## Smart commands (v0.3)

```bash
# Lineage
omd lineage svc.db.schema.orders                        # ascii tree, both directions
omd lineage svc.db.schema.orders --up --depth 3         # upstream only
omd lineage svc.db.schema.orders --format mermaid       # for docs / diagrams
omd lineage svc.db.schema.orders --format dot | dot -Tpng -o l.png

# Edit fields (fetches entity, computes JSON patch, sends it)
omd edit svc.db.schema.orders --description @desc.md
omd edit svc.db.schema.orders --display-name "Orders"
omd edit svc.db.schema.orders --owner some.user         # FQN of user or team
omd edit svc.db.schema.orders --tier Tier.Tier1
omd edit ... --dry-run                                  # print patch, don't send

# Tags
omd tag svc.db.schema.orders --add PII.Sensitive --add Tier.Tier2
omd tag svc.db.schema.orders --remove PII.Sensitive

# Glossary
omd glossary assign svc.db.schema.orders --term CustomerData.Email

# Data quality
omd quality list --table svc.db.schema.orders
omd quality results <test-case-fqn> --limit 20
omd quality latest svc.db.schema.orders

# Shell completions
omd completions bash > /etc/bash_completion.d/omd
omd completions zsh  > ~/.config/zsh/functions/_omd
omd completions fish > ~/.config/fish/completions/omd.fish
```

## CSV import / export (v0.4)

```bash
# Export an entity's metadata as CSV
omd export table svc.db.schema.orders -o orders.csv
omd export glossary Retail -o retail.csv
omd export databaseSchema svc.db.schema -o schema.csv

# Dry-run an import (default) to see what would change
omd import table svc.db.schema.orders updates.csv

# Commit the changes
omd import table svc.db.schema.orders updates.csv --apply

# Pipe a CSV from another tool
some-generator | omd import table svc.db.schema.orders -
```

Supported entity types: `table`, `database`, `databaseSchema`, `glossary`,
`glossaryTerm`, `team`, `user`, `databaseService`, `securityService`,
`driveService`, `testCase`.

Environment overrides:

- `OMD_HOST` — server URL
- `OMD_TOKEN` — JWT bearer token
- `OMD_PROFILE` — which saved profile to use
- `OMD_LOG` — tracing filter (`info`, `debug`, etc.)

## Configuration

Stored in `~/.omd/`:

- `config.toml` — host, timeout, per-profile settings
- `credentials` — JWT tokens (mode 0600)

## Roadmap

- v0.1 configure, JWT auth, search, describe, raw, sync
- v0.2 dynamic command generation from OpenAPI spec
- v0.3 lineage, quality, edit, tag, glossary, completions
- v0.4 CSV import/export
- v0.5 MCP server mode
- v0.6 SSO login (OIDC/PKCE)
- v0.7 release automation, installer
- v1.0 stable

## License

Apache-2.0
