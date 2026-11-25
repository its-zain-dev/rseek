# rseek

Ultra-lightweight, completely free, zero-API-key metasearch engine written in **Rust**. Runs locally in your terminal, or serves a fast JSON REST API, Web UI, and OpenSearch descriptor on any server or container.

## Features

- **100% Free & Zero API Keys:** Concurrent search across 10 built-in sources (`duckduckgo`, `wikipedia`, `wiby`, `hackernews`, `github`, `stackexchange`, `arxiv`, `crates`, `npm`, `wikimedia`) plus DuckDuckGo Instant Answer infoboxes.
- **Smart Ranking & Deduplication:** Weighted Reciprocal Rank Fusion (RRF), multi-engine consensus boost, query-term relevance scoring, and canonical URL deduplication.
- **Lightweight Single Binary:** Static musl & Android/Termux builds, pure-Rust `rustls` TLS, system DNS + automatic cached DNS-over-HTTPS (DoH) fallback for Termux/Android, bounded in-memory TTL cache, and per-engine circuit breakers.
- **Terminal CLI + Interactive REPL:** Pretty ANSI output, `compact`, `json`, `markdown`, and `urls` formats, `!bang` shortcuts, or query a remote hosted `rseek` instance with `--server`.
- **Self-Hostable HTTP API & Web UI:** Built-in responsive dark/light Web UI (`/`), REST API (`/api/v1/search`), SearXNG-compatible `/search` endpoint, `/api/v1/suggest`, `/api/v1/engines`, `/api/v1/health`, `/opensearch.xml`, optional API-key auth, CORS, and per-IP rate limiting.

## Quick Install (Prebuilt Binaries)

Download from [GitHub Releases](https://github.com/its-zain-dev/rseek/releases):

```bash
# Termux / Android (aarch64)
curl -fsSL https://github.com/its-zain-dev/rseek/releases/latest/download/rseek-aarch64-linux-android.tar.gz | tar -xz -C ~/.local/bin

# Linux x86_64 (static musl)
curl -fsSL https://github.com/its-zain-dev/rseek/releases/latest/download/rseek-x86_64-unknown-linux-musl.tar.gz | tar -xz -C /usr/local/bin

# Linux ARM64 (static musl)
curl -fsSL https://github.com/its-zain-dev/rseek/releases/latest/download/rseek-aarch64-unknown-linux-musl.tar.gz | tar -xz -C /usr/local/bin
```

## Terminal CLI Usage

```bash
# Quick search
rseek "rust async runtime"

# Category search
rseek -c code "axum middleware"
rseek -c science "transformer attention"
rseek -c packages "http client"

# Bang shortcuts (leading or trailing)
rseek "!w linux kernel"
rseek "!gh tokio-rs/axum"
rseek "!so rust lifetime"
rseek "!arxiv diffusion models"
rseek "!crates serde"
rseek "!npm react"
rseek "!hn webassembly"
rseek "!img mountain"

# Script-friendly output formats
rseek --json -n 5 "rust search"
rseek --format markdown "rust search"
rseek --format urls "rust search"
rseek --format compact "rust search"

# Interactive REPL
rseek repl

# List all engines and bangs
rseek engines

# Query a remote hosted rseek instance from your terminal
rseek --server https://search.example.com --api-key mysecret "rust"
```

## Supported `!bangs`

| Bang | Target |
| --- | --- |
| `!g`, `!ddg` | DuckDuckGo Web |
| `!w`, `!wiki` | Wikipedia |
| `!wiby` | Wiby Small Web |
| `!hn` | Hacker News |
| `!gh`, `!github` | GitHub Repositories |
| `!so` | Stack Overflow |
| `!arxiv`, `!paper` | arXiv Papers |
| `!crates`, `!crate`, `!rs` | crates.io |
| `!npm`, `!js` | npm Registry |
| `!img`, `!images` | Wikimedia Commons |
| `!code`, `!news`, `!sci`, `!pkg`, `!all` | Category switch |

## HTTP API & Hosting

Start locally or on a public host:

```bash
# Localhost (Web UI + API on http://127.0.0.1:8080)
rseek serve

# Public host with API key and rate limiting
rseek serve --host 0.0.0.0 --port 8080 --api-key "your-secret" --rate-limit 60
```

Or using Docker Compose:

```bash
docker compose up -d --build
```

### Endpoints

- `GET /` — Embedded Web UI
- `GET /api/v1/search?q=<query>&category=general&page=1&limit=15&lang=en` — Metasearch JSON response
- `GET /search?q=<query>&format=json` — SearXNG-compatible search route
- `GET /api/v1/suggest?q=<query>` — Autocomplete suggestions
- `GET /api/v1/engines` — Registered engines & live circuit-breaker metrics
- `GET /api/v1/health` — Service health, uptime, and cache count
- `GET /opensearch.xml` — Browser OpenSearch descriptor

When `--api-key` (`RSEEK_API_KEY`) is configured, pass it via `X-API-Key: <key>`, `Authorization: Bearer <key>`, or `?api_key=<key>`.

## License

MIT — see [LICENSE](LICENSE).
