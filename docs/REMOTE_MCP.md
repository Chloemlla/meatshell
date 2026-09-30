# Authenticated remote MCP (Linux or Windows)

`meatshell mcp serve` remains the existing local stdio transport. To run a
remote service, use the **new, opt-in** `--http-config` mode. It serves the
standard MCP Streamable HTTP protocol at `/mcp`, using the official Rust MCP
SDK. Both desktop and `--features headless` builds include this mode. Running
without arguments still starts the GUI in the default desktop build.

This is an **OAuth resource server**, not an identity provider. It verifies
RS256 access tokens from an established external OAuth 2.1 / OIDC provider.
You must configure that provider and an HTTPS reverse proxy before connecting
ChatGPT/dot. No account, token, signing key, tunnel, public listener or production
profile is created by installing the release. Local synthetic integration tests
do not constitute an end-to-end ChatGPT login test.

## 1. Prepare one deliberately selected profile

Run under a dedicated unprivileged OS account. Create a private profile directory
(mode 0700 on Linux) and select it explicitly with `--data-dir` or
`MEATSHELL_DATA_DIR`. HTTP mode refuses to use an implicit default/sidecar profile.
You can import an export using the existing CLI, or configure that profile in
the desktop application. Never place profile files, credentials or SSH keys in
web roots or release packages.

Existing profile settings remain authoritative:

- `mcp_enabled` must be enabled
- Saved credential use requires `mcp_use_saved_credentials`
- Arbitrary SSH commands require `mcp_allow_commands`
- SFTP transfers/imports require `mcp_allow_file_transfers`
- Applying configuration imports additionally requires startup flag
  `--allow-config-import` (before `--http-config`)
- Unknown/changed SSH host keys fail closed. Establish the intended known_hosts
  entries through the existing trusted workflow; never bypass this check

OAuth authorization adds an outer boundary; it does **not** enable these gates.
Authorized subjects all access the same selected profile and the OS user's
filesystem permissions. This is a **single-owner / trusted-operator** service,
not a multi-tenant SSH hosting service. For separate users/data, run separate
OS accounts, profiles and service instances. A subject allowlist is mandatory.

## 2. Configure the external authorization server

Use an established provider with authorization-code + PKCE S256, issuer
metadata discovery and a ChatGPT-compatible client registration mechanism
(CIMD, DCR, or a predefined client). Configure the exact redirect URI displayed
by the ChatGPT connection setup. Configure it to issue **RS256 access tokens**
with:

- `iss`: exact configured HTTPS issuer, including any trailing slash
- `aud`: exact canonical resource URL, e.g. `https://shell.example.com/mcp`
- `sub`: the explicitly allowed account identifier
- `exp`: a mandatory expiry timestamp; `nbf` is checked when present
- `scope`: a space-separated string containing `meatshell:mcp`

The provider must honor the OAuth `resource` parameter. Do not use ID tokens,
shared static bearer tokens, a client secret, or SSH credentials as access tokens.
Static bearer authentication is not implemented by this HTTP adapter.

Download the provider's **public** JWKS through a trusted, TLS-verified admin
workflow into a local `public-jwks.json` file. The service deliberately does not
fetch URLs supplied by clients. Only RSA/RS256 signature-verification keys with
unique nonempty `kid` values are usable; private/symmetric key material is rejected.
Protect the file against untrusted modification. On rotation, replace it
atomically with the provider's current public keys and restart the service.
For overlap, retain both old and new public keys until old access tokens expire.
Key removal, subject/scope changes and revocation require restarting this service;
JWT revocation/introspection is not supported. Use short token lifetimes.

See the official [MCP authorization specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)
and [OpenAI authentication requirements](https://developers.openai.com/plugins/build/auth).

## 3. Create the service configuration

Example `http.json` (all hostnames and subjects below are placeholders):

```json
{
  "bind": "127.0.0.1:8765",
  "allowed_origins": [],
  "oauth": {
    "issuer": "https://identity.example.com/",
    "resource": "https://shell.example.com/mcp",
    "jwks_file": "public-jwks.json",
    "required_scope": "meatshell:mcp",
    "allowed_subjects": ["REPLACE_WITH_YOUR_PROVIDER_SUBJECT"]
  }
}
```

`jwks_file` is relative to the configuration file (or absolute). The default bind
is loopback `127.0.0.1:8765`. Authentication is mandatory for every MCP request,
even loopback requests; there is no unauthenticated/public-bind flag. A non-loopback
bind is supported only for a private, firewalled proxy network; the application
speaks HTTP internally, so do not expose that listener directly to the Internet.

Every request with an `Origin` header must match an explicitly listed HTTPS
origin. With the empty default, all browser-origin requests are rejected; normal
server-to-server MCP requests do not need an Origin. No wildcard CORS is enabled.
Host must match the configured resource authority or bind address. The reverse
proxy must preserve the public Host header. Forwarded headers never grant trust.

Start (example paths are operator-selected, not shipped profiles):

```sh
meatshell --data-dir /var/lib/meatshell/profile mcp serve --http-config /etc/meatshell/http.json
```

No access tokens/passwords belong in command-line arguments or logs. Clients
send access tokens solely using the HTTP `Authorization: Bearer ...` header.

## 4. Terminate HTTPS in a reverse proxy

Example Caddy configuration (install/manage Caddy separately from its official
source; DNS/TLS setup is the operator's responsibility):

```caddyfile
shell.example.com {
    reverse_proxy 127.0.0.1:8765
}
```

Proxy both `/mcp` and `/.well-known/oauth-protected-resource*`. Preserve
Authorization and Host headers, disable buffering of SSE, set suitable request
and idle timeouts, and do not log Authorization or request bodies. Restrict the
upstream HTTP port at the firewall. Configure proxy-level connection/rate limits
and TLS, including protection against slow HTTP headers before requests reach
application middleware.

Optional systemd unit (adjust all paths/user names; this does not install itself):

```ini
[Unit]
Description=MeatShell authenticated MCP
After=network-online.target
[Service]
User=meatshell
Group=meatshell
ExecStart=/opt/meatshell/meatshell --data-dir /var/lib/meatshell/profile mcp serve --http-config /etc/meatshell/http.json
WorkingDirectory=/var/lib/meatshell
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/var/lib/meatshell
Restart=on-failure
TimeoutStopSec=15
[Install]
WantedBy=multi-user.target
```

Filesystem hardening may intentionally prevent upload/download/import outside the
profile. Grant only the paths this service actually needs. Never run as root.
On Windows, use equivalent private directory ACLs and a service manager. Native
Windows packaging/build results are stated separately in each release; a Linux
runtime test is not Windows runtime verification.

## 5. Connect and verify

The public URL is `https://shell.example.com/mcp`. Unauthenticated `/mcp`
requests return 401 with a `WWW-Authenticate` discovery challenge. Public metadata
at `/.well-known/oauth-protected-resource/mcp` (also the root well-known path)
contains only resource/issuer/scope information, never sessions or credentials.

Use MCP Inspector, then [ChatGPT's connection setup](https://developers.openai.com/plugins/deploy/connect-chatgpt).
The external provider, redirect URI, scopes, audience, HTTPS certificate and
public reachability must work together. This release has no preconfigured dot
plugin or production deployment. Do not claim it is connected until that end-to-end
flow succeeds in the target environment.

## Runtime limits and cancellation

- RS256 signature, issuer, audience, expiry/not-before, scope and subject checked
  on every request; invalid tokens return 401, insufficient permission 403
- MCP session IDs bind to the authenticated subject of the configured issuer;
  IDs belonging to another subject/unknown/expired IDs all return 404
- At most 64 protocol sessions, 15-minute absolute lifetime, SDK idle cleanup;
  clients must reinitialize after expiry. Refreshing an OAuth token for the same
  subject can continue a live session; subject changes cannot
- 1 MiB HTTP body limit, 5-second body-read deadline, 10-second protocol-header
  response deadline, 16 tool calls/response streams; control notifications retain
  separate admission capacity
- Tool calls and response streams end after at most 300 seconds or token expiry,
  whichever occurs first. Existing per-operation limits still apply
- Use `notifications/cancelled` to cancel an in-flight request, and HTTP DELETE to
  close its MCP session. Cancellation stops local SSH/SFTP work; it cannot undo
  commands already executed, bytes already written, or remote jobs detached by a
  command. A dropped network connection is not proof an action was undone
- Results use POST SSE response streams. Standalone GET streams/resumption are
  deliberately unsupported (405); completed-response replay caching is disabled
- SIGTERM/Ctrl-C requests graceful shutdown. Configuration/keys are loaded once
  per process; restart after changes

## Reproducible checks

```sh
cargo test --features headless --bin meatshell
cargo build --features headless
python3 tests/remote_mcp_e2e.py --exe target/debug/meatshell
python3 tests/config_import_e2e.py --exe target/debug/meatshell
# Additional loopback SSH/SFTP tests require Python paramiko:
python3 tests/ssh_jump_chain_e2e.py --exe target/debug/meatshell --stage-timeouts
cargo check                         # default desktop source compatibility
```

Tests generate synthetic RSA signing keys in memory, write only their public
JWKS, create disposable profiles and connect solely to loopback fixtures.
A headless archive contains CLI/MCP functionality and no GUI; desktop archives
keep the GUI, CLI, stdio MCP and HTTP MCP in the same executable.
