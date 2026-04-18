# omd — OpenMetadata CLI

Command-line tool for [OpenMetadata](https://github.com/open-metadata/OpenMetadata). Written in Rust.

Dynamic command surface generated from OpenMetadata's OpenAPI spec, plus hand-tuned "smart" commands for the common workflows (search, describe, lineage, quality, CSV import/export). Structured JSON output for scripts and agents. Ships an MCP server mode (`omd mcp`) so AI agents can drive OpenMetadata directly.

Status: **v0.7**.

## Install

Pick whichever fits your machine.

### Script (Linux, macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/Allen-Saji/openmetadata-cli/main/install.sh | sh
```

Installs the latest release binary to `~/.local/bin` (override with
`INSTALL_DIR=/usr/local/bin`). Pin a version with `OMD_VERSION=v0.7.0`.

### Homebrew

```bash
brew tap Allen-Saji/tap
brew install omd
```

### Cargo

```bash
cargo install openmetadata-cli            # publishes the `omd` binary
```

### Manual

Grab a tarball for your platform from
[GitHub Releases](https://github.com/Allen-Saji/openmetadata-cli/releases/latest)
and extract `omd` onto your `PATH`. Each asset ships a `.sha256` next to it
for verification.

### From source

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

## MCP server (v0.5)

`omd mcp` starts a Model Context Protocol server on stdio so AI agents
(Claude Desktop, Cursor, Claude Code, etc.) can drive OpenMetadata through
a curated tool surface.

Exposed tools: `search`, `resolve_fqn`, `describe_entity`, `get_lineage`,
`list_upstream`, `list_downstream`, `update_description`, `add_tag`,
`remove_tag`, `assign_glossary_term`, `list_quality_tests`,
`get_test_results`, `export_csv`, `import_csv`. An opt-in `raw_request`
tool is available when the server is started with `OMD_MCP_ALLOW_RAW=1`.

### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`
(macOS) or the equivalent on other platforms:

```json
{
  "mcpServers": {
    "openmetadata": {
      "command": "omd",
      "args": ["mcp"],
      "env": {
        "OMD_HOST": "https://your-openmetadata-host",
        "OMD_TOKEN": "your-jwt"
      }
    }
  }
}
```

### Cursor

Add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "openmetadata": {
      "command": "omd",
      "args": ["mcp"]
    }
  }
}
```

With `omd configure` + `omd auth login` already run, no env vars are needed.

## SSO login (v0.6)

For OpenMetadata servers with OIDC-backed SSO (Google, Okta, Azure AD, Auth0, ...):

```bash
omd auth login --sso
# opens the browser, completes the OIDC flow, saves the id_token
```

Behind the scenes:

1. Reads the server's public auth config at `/api/v1/system/config/auth` to discover the provider and authority.
2. Discovers the authorize/token endpoints via `.well-known/openid-configuration`.
3. Runs the OIDC authorization-code flow with PKCE (S256), binding a random loopback port for the callback.
4. Stores the resulting token the same way `--token` would.

Orgs that register a separate public OIDC client for CLIs (like `kubectl`/`gh`) can override the client:

```bash
omd auth login --sso --client-id <public-client-id>
omd auth login --sso --authority https://custom-idp.example.com
omd auth login --sso --scopes "openid email profile offline_access"
```

SAML is intentionally not supported. SAML users can still paste a JWT directly with `omd auth login --token ...`.

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
- v0.5 MCP server mode (rmcp, 14 curated tools)
- v0.6 SSO login (OIDC/PKCE with browser loopback)
- v0.7 release automation (install.sh, CI, tag-triggered cross-platform builds, Homebrew draft)
- v1.0 stable

## License

Apache-2.0
