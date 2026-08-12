# Lungyam (ลุงยาม) 🛡️

![Rust](https://img.shields.io/badge/Rust-2024-black?style=flat-square&logo=rust&logoColor=white)
![Pingora](https://img.shields.io/badge/Pingora-0.8-blue?style=flat-square)
![CI](https://img.shields.io/github/actions/workflow/status/phongsathornpt/lungyam/ci.yml?style=flat-square&label=CI)
![License](https://img.shields.io/badge/license-MIT-green?style=flat-square)

**Lungyam** is a small, configuration-driven API gateway and reverse proxy written in Rust on top of Pingora.

ลุงยามถูกออกแบบให้เป็นหน้าด่านของ backend services โดยเน้น latency ต่ำ, resource footprint เล็ก, routing ที่กำหนดด้วย config และ policy ที่ทำงานก่อน/หลัง proxy request.

## Status

The first native/container MVP is implemented. It currently supports HTTP upstreams and is intended as the foundation for later edge-runtime adapters.

## Features

- Pingora-based streaming HTTP reverse proxy
- YAML configuration with startup validation
- Host, path, and HTTP method routing
- Explicit route priority and path-specificity ordering
- Multiple upstream endpoints with health-aware round-robin selection
- Active TCP health checks for upstream pools
- Bounded connection failover across upstream endpoints
- Upstream connect/read/write timeouts
- Request and response header transforms
- Request IDs propagated to upstreams and responses
- Structured access logging through the Rust `log` facade
- Local fixed-window rate limiting
- `Content-Length` request-size guard
- Built-in `GET /health` endpoint
- Docker and Docker Compose examples
- CI covering formatting, build, unit tests, Clippy, and end-to-end proxy behavior

## Architecture

```text
Client
  |
  v
Lungyam / Pingora
  |
  +-- health check shortcut
  +-- route match (host -> path -> method)
  +-- request size guard
  +-- local rate limit
  +-- request header transforms
  +-- request id / route metadata
  |
  v
Health-aware upstream pool
  |
  +-- TCP health checker
  +-- round-robin healthy endpoint selection
  +-- bounded retry on connection failure
  |
  v
Backend response
  |
  +-- response header transforms
  +-- request id
  +-- access log
  |
  v
Client
```

The Cargo workspace is split into three crates:

```text
crates/
├── lungyam-core/   # configuration/domain model and validation
├── lungyam-proxy/  # Pingora native data plane
└── lungyam-cli/    # command-line entry point
```

## Requirements

- Rust stable with Edition 2024 support
- `curl` and Python 3 only when running the repository integration test
- Docker / Docker Compose are optional

## Quick start

```bash
git clone https://github.com/phongsathornpt/lungyam.git
cd lungyam
cargo build --release
```

Start a backend on `127.0.0.1:3000`, then run Lungyam:

```bash
RUST_LOG=info cargo run -p lungyam-cli -- --config config/lungyam.yaml
```

Check the gateway itself:

```bash
curl http://127.0.0.1:8080/health
```

Expected response:

```text
ok
```

## Configuration

The default example lives at `config/lungyam.yaml`.

```yaml
server:
  listen: 0.0.0.0:8080

upstreams:
  app:
    endpoints:
      - 127.0.0.1:3000
      - 127.0.0.1:3001
    connect_timeout_ms: 3000
    read_timeout_ms: 30000
    write_timeout_ms: 30000
    health_check_interval_seconds: 5

routes:
  - name: app
    host: api.example.com
    path: /api
    methods: [GET, POST]
    upstream: app
    priority: 100
    policies:
      request_headers:
        remove: [x-remove-me]
        add:
          x-lungyam-proxy: lungyam
      response_headers:
        add:
          x-lungyam: edge
      rate_limit:
        requests: 100
        window_seconds: 60
      max_request_body_bytes: 1048576
```

A route matches when all configured constraints match. `host` and `methods` are optional. Path matching respects segment boundaries, so `/api` matches `/api` and `/api/users` but not `/apiv2`. Higher `priority` wins; when priorities are equal, the longer path is evaluated first.

Configuration is validated before the server starts. Invalid upstream references, empty upstream pools, duplicate route names, malformed paths, zero health-check intervals, and invalid rate-limit values are rejected.

Each upstream pool is checked with a TCP health probe. `health_check_interval_seconds` defaults to `5` when omitted. Only healthy endpoints are selected once health state is known. If a selected endpoint still fails during connection establishment, Lungyam marks the connection error retryable while another endpoint remains, preventing an unbounded retry loop.

## Header transforms

Request and response policies can remove headers and then add or replace headers:

```yaml
policies:
  request_headers:
    remove: [x-internal]
    add:
      x-proxied-by: lungyam
  response_headers:
    add:
      x-edge: lungyam
```

Lungyam also adds `x-request-id` to proxied requests and responses and `x-lungyam-route` to proxied requests.

## Run tests

Run the Rust quality checks:

```bash
cargo fmt --all -- --check
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Run the end-to-end test:

```bash
bash tests/integration.sh
```

The integration test starts a fixture backend and verifies method, path, query string, request body, request/response header transforms, request IDs, route matching, health-aware upstream failover, health endpoint behavior, and request-size rejection through a real Lungyam process.

## Docker Compose

A development stack with the integration fixture backend is included:

```bash
docker compose up --build
```

Lungyam listens on `http://127.0.0.1:8080` and proxies to the fixture backend service inside the Compose network.

## Current limitations

The MVP intentionally keeps the data plane small:

- Upstream connections are currently plain HTTP; upstream TLS configuration is not exposed yet.
- Rate limiting is local to one Lungyam process and keyed by route, not client identity.
- The request-size guard rejects oversized requests when `Content-Length` is present; a streaming byte counter is not implemented yet.
- Failover currently targets connection-establishment failures; automatic retry on backend HTTP 5xx responses is not enabled.
- Authentication, distributed rate limiting, caching, WAF rules, and WASM edge adapters are future work.

## License

MIT. See `LICENSE`.
