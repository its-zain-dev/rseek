pub mod repl;

use std::io::IsTerminal;
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::config::{SearchConfig, ServerConfig};
use crate::dns::build_http_client;
use crate::engine::SearchEngineService;
use crate::models::{Category, EngineInfo, EngineStatus, SearchQuery, SearchResponse, SuggestResponse};
use crate::server;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Pretty,
    Compact,
    Json,
    Markdown,
    Urls,
}

#[derive(Parser, Debug)]
#[command(
    name = "rseek",
    version,
    about = "Ultra-lightweight, free Rust metasearch engine (CLI, Terminal REPL, HTTP API & Web UI)"
)]
pub struct Cli {
    /// Search query (shorthand for `rseek search <QUERY>`)
    #[arg(value_name = "QUERY")]
    pub query: Vec<String>,

    /// Search category
    #[arg(short = 'c', long = "category", default_value = "general", global = true)]
    pub category: Category,

    /// Comma-separated engines to query (e.g. duckduckgo,wikipedia,github)
    #[arg(short = 'e', long = "engines", value_delimiter = ',', global = true)]
    pub engines: Vec<String>,

    /// Language code (e.g. en, de, fr, ar)
    #[arg(short = 'l', long = "lang", default_value = "en", global = true)]
    pub lang: String,

    /// Page number (1-indexed)
    #[arg(short = 'p', long = "page", default_value_t = 1, global = true)]
    pub page: u32,

    /// Maximum results to return
    #[arg(short = 'n', long = "limit", default_value_t = 15, global = true)]
    pub limit: usize,

    /// Per-engine timeout in milliseconds
    #[arg(
        long = "timeout-ms",
        default_value_t = 5000,
        env = "RSEEK_TIMEOUT_MS",
        global = true
    )]
    pub timeout_ms: u64,

    /// Output format
    #[arg(short = 'f', long = "format", default_value = "pretty", global = true)]
    pub format: OutputFormat,

    /// Shorthand for --format json
    #[arg(long = "json", global = true)]
    pub json: bool,

    /// Bypass in-memory search cache
    #[arg(long = "no-cache", global = true)]
    pub no_cache: bool,

    /// Disable ANSI terminal colors
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Query a remote rseek server URL instead of running locally (e.g. http://127.0.0.1:8080)
    #[arg(long = "server", env = "RSEEK_SERVER", global = true)]
    pub server: Option<String>,

    /// API key for server mode or remote --server requests
    #[arg(long = "api-key", env = "RSEEK_API_KEY", global = true)]
    pub api_key: Option<String>,

    /// HTTP/HTTPS/SOCKS proxy URL for upstream requests
    #[arg(long = "proxy", env = "RSEEK_PROXY", global = true)]
    pub proxy: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run a metasearch query from the terminal
    Search {
        #[arg(required = true, value_name = "QUERY")]
        query: Vec<String>,
    },
    /// Start the HTTP API server and embedded Web UI
    Serve {
        /// Bind host address
        #[arg(long = "host", default_value = "127.0.0.1", env = "RSEEK_HOST")]
        host: String,

        /// Bind TCP port
        #[arg(long = "port", default_value_t = 8080, env = "RSEEK_PORT")]
        port: u16,

        /// Access-Control-Allow-Origin header value
        #[arg(long = "cors-origin", default_value = "*", env = "RSEEK_CORS_ORIGIN")]
        cors_origin: String,

        /// Max API requests per IP per minute (0 disables rate limiting)
        #[arg(long = "rate-limit", default_value_t = 60, env = "RSEEK_RATE_LIMIT")]
        rate_limit_per_min: u32,

        /// Cache TTL in seconds (0 disables caching)
        #[arg(long = "cache-ttl", default_value_t = 300, env = "RSEEK_CACHE_TTL")]
        cache_ttl_secs: u64,

        /// Maximum cached search responses
        #[arg(long = "cache-max", default_value_t = 500, env = "RSEEK_CACHE_MAX")]
        cache_max_entries: usize,
    },
    /// Launch interactive terminal search REPL
    Repl,
    /// List available search engines and their bangs
    Engines,
    /// Fetch autocomplete query suggestions
    Suggest {
        #[arg(required = true, value_name = "QUERY")]
        query: Vec<String>,
    },
}

/// Execution context supporting both local in-process search and remote `--server` queries.
pub enum CliContext {
    Local(Arc<SearchEngineService>),
    Remote {
        base_url: String,
        api_key: Option<String>,
        client: reqwest::Client,
    },
}

impl CliContext {
    pub async fn search(&self, req: SearchQuery) -> Result<SearchResponse, String> {
        match self {
            Self::Local(service) => Ok(service.search(req).await),
            Self::Remote {
                base_url,
                api_key,
                client,
            } => {
                let url = format!("{}/api/v1/search", base_url.trim_end_matches('/'));
                let mut builder = client.get(url).query(&[
                    ("q", req.query.as_str()),
                    ("category", req.category.as_str()),
                    ("lang", req.lang.as_str()),
                    ("page", &req.page.to_string()),
                    ("limit", &req.limit.to_string()),
                    ("timeout_ms", &req.timeout_ms.to_string()),
                    ("no_cache", &req.no_cache.to_string()),
                ]);
                if !req.engines.is_empty() {
                    builder = builder.query(&[("engines", req.engines.join(","))]);
                }
                if let Some(key) = api_key {
                    builder = builder.header("X-API-Key", key);
                }
                let resp = builder
                    .send()
                    .await
                    .map_err(|e| format!("remote search request failed: {e}"))?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("server returned {status}: {body}"));
                }
                resp.json::<SearchResponse>()
                    .await
                    .map_err(|e| format!("invalid JSON from server: {e}"))
            }
        }
    }

    pub async fn engines(&self) -> Result<Vec<EngineInfo>, String> {
        match self {
            Self::Local(service) => Ok(service.engine_infos().await),
            Self::Remote {
                base_url,
                api_key,
                client,
            } => {
                let url = format!("{}/api/v1/engines", base_url.trim_end_matches('/'));
                let mut builder = client.get(url);
                if let Some(key) = api_key {
                    builder = builder.header("X-API-Key", key);
                }
                let resp = builder
                    .send()
                    .await
                    .map_err(|e| format!("remote engines request failed: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("server returned {}", resp.status()));
                }
                let val: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| format!("invalid JSON from server: {e}"))?;
                serde_json::from_value(val["engines"].clone())
                    .map_err(|e| format!("invalid engines payload: {e}"))
            }
        }
    }

    pub async fn suggest(&self, query: &str, lang: &str) -> Result<SuggestResponse, String> {
        match self {
            Self::Local(service) => Ok(service.suggest(query, lang).await),
            Self::Remote {
                base_url,
                api_key,
                client,
            } => {
                let url = format!("{}/api/v1/suggest", base_url.trim_end_matches('/'));
                let mut builder = client.get(url).query(&[("q", query), ("lang", lang)]);
                if let Some(key) = api_key {
                    builder = builder.header("X-API-Key", key);
                }
                let resp = builder
                    .send()
                    .await
                    .map_err(|e| format!("remote suggest request failed: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("server returned {}", resp.status()));
                }
                resp.json::<SuggestResponse>()
                    .await
                    .map_err(|e| format!("invalid JSON from server: {e}"))
            }
        }
    }
}

pub async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let format = if cli.json {
        OutputFormat::Json
    } else {
        cli.format
    };
    let no_color = cli.no_color || std::env::var_os("NO_COLOR").is_some();

    if let Some(Command::Serve {
        host,
        port,
        cors_origin,
        rate_limit_per_min,
        cache_ttl_secs,
        cache_max_entries,
    }) = cli.command
    {
        let search_cfg = SearchConfig {
            timeout_ms: cli.timeout_ms,
            cache_ttl_secs,
            cache_max_entries,
            proxy: cli.proxy,
            ..SearchConfig::default()
        };
        let service = Arc::new(SearchEngineService::new(search_cfg)?);
        let server_cfg = ServerConfig {
            host,
            port,
            api_key: cli.api_key,
            cors_origin,
            rate_limit_per_min,
        };
        return server::serve(service, server_cfg).await;
    }

    let ctx = if let Some(server_url) = cli.server {
        let client = build_http_client(cli.timeout_ms, cli.proxy.as_deref())?;
        CliContext::Remote {
            base_url: server_url,
            api_key: cli.api_key,
            client,
        }
    } else {
        let search_cfg = SearchConfig {
            timeout_ms: cli.timeout_ms,
            proxy: cli.proxy,
            ..SearchConfig::default()
        };
        CliContext::Local(Arc::new(SearchEngineService::new(search_cfg)?))
    };

    match cli.command {
        Some(Command::Search { query }) => {
            run_query_cmd(&ctx, &cli, query.join(" "), format, no_color).await
        }
        Some(Command::Engines) => {
            let infos = ctx.engines().await?;
            render_engines(&infos, format, no_color);
            Ok(())
        }
        Some(Command::Suggest { query }) => {
            let resp = ctx.suggest(&query.join(" "), &cli.lang).await?;
            if format == OutputFormat::Json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&resp).unwrap_or_default()
                );
            } else {
                for s in resp.suggestions {
                    println!("{s}");
                }
            }
            Ok(())
        }
        Some(Command::Repl) => {
            repl::run_repl(&ctx, cli.category, cli.engines, cli.lang, cli.limit, no_color).await
        }
        Some(Command::Serve { .. }) => unreachable!(),
        None => {
            if !cli.query.is_empty() {
                run_query_cmd(&ctx, &cli, cli.query.join(" "), format, no_color).await
            } else if std::io::stdin().is_terminal() {
                repl::run_repl(&ctx, cli.category, cli.engines, cli.lang, cli.limit, no_color).await
            } else {
                let mut cmd = Cli::command();
                cmd.print_help().map_err(|e| e.to_string())?;
                println!();
                Ok(())
            }
        }
    }
}

async fn run_query_cmd(
    ctx: &CliContext,
    cli: &Cli,
    query_str: String,
    format: OutputFormat,
    no_color: bool,
) -> Result<(), String> {
    let req = SearchQuery {
        query: query_str,
        category: cli.category,
        engines: cli.engines.clone(),
        lang: cli.lang.clone(),
        page: cli.page,
        limit: cli.limit,
        timeout_ms: cli.timeout_ms,
        no_cache: cli.no_cache,
    };
    let resp = ctx.search(req).await?;
    render_search_response(&resp, format, no_color);
    Ok(())
}

/// Prints a [`SearchResponse`] to stdout in the requested terminal format.
pub fn render_search_response(resp: &SearchResponse, format: OutputFormat, no_color: bool) {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(resp).unwrap_or_else(|_| "{}".to_string())
            );
        }
        OutputFormat::Urls => {
            for r in &resp.results {
                println!("{}", r.url);
            }
        }
        OutputFormat::Markdown => {
            println!("# Search: `{}` ({} results, {}ms)\n", resp.query, resp.total_results, resp.took_ms);
            if let Some(info) = &resp.infobox {
                println!("> **{}** — {} ([{}]({}))\n", info.title, info.abstract_text, info.source, info.url);
            }
            for (i, r) in resp.results.iter().enumerate() {
                println!("{}. **[{}]({})**", i + 1, r.title, r.url);
                if !r.snippet.is_empty() {
                    println!("   {}", r.snippet);
                }
                println!("   *Engines: {} | Score: {}*\n", r.engines.join(", "), r.score);
            }
        }
        OutputFormat::Compact => {
            for (i, r) in resp.results.iter().enumerate() {
                println!("{}. {} — {}", i + 1, r.title, r.url);
            }
        }
        OutputFormat::Pretty => {
            let use_color = !no_color && std::io::stdout().is_terminal();
            let (bold, cyan, green, dim, yellow, reset) = if use_color {
                ("\x1b[1m", "\x1b[36m", "\x1b[32m", "\x1b[2m", "\x1b[33m", "\x1b[0m")
            } else {
                ("", "", "", "", "", "")
            };

            let engine_summary: Vec<String> = resp
                .engines
                .iter()
                .map(|e| {
                    let tag = match e.status {
                        EngineStatus::Ok => format!("{} ({})", e.name, e.results),
                        EngineStatus::Empty => format!("{}:empty", e.name),
                        EngineStatus::Timeout => format!("{}:timeout", e.name),
                        EngineStatus::RateLimited => format!("{}:rate-limited", e.name),
                        EngineStatus::CircuitOpen => format!("{}:cooldown", e.name),
                        EngineStatus::Error => format!("{}:err", e.name),
                    };
                    tag
                })
                .collect();

            println!(
                "{bold}rseek{reset} · \"{}\" [{yellow}{}{reset}] · {} results in {}ms{} {dim}[{}]{reset}",
                resp.effective_query,
                resp.category,
                resp.total_results,
                resp.took_ms,
                if resp.cached { " (cached)" } else { "" },
                engine_summary.join(", ")
            );

            if let Some(info) = &resp.infobox {
                println!("\n{bold}[Infobox: {}]{reset}", info.title);
                println!("  {}", info.abstract_text);
                println!("  {green}{}{reset} ({})", info.url, info.source);
            }

            if resp.results.is_empty() {
                println!("\nNo results found.");
                return;
            }

            println!();
            for (i, r) in resp.results.iter().enumerate() {
                println!("{bold}{}. {cyan}{}{reset}", i + 1, r.title);
                println!("   {green}{}{reset}", r.url);
                if !r.snippet.is_empty() {
                    println!("   {}", r.snippet);
                }
                println!(
                    "   {dim}engines: [{}]  score: {}{reset}\n",
                    r.engines.join(", "),
                    r.score
                );
            }
        }
    }
}

fn render_engines(infos: &[EngineInfo], format: OutputFormat, no_color: bool) {
    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "engines": infos }))
                .unwrap_or_default()
        );
        return;
    }

    let use_color = !no_color && std::io::stdout().is_terminal();
    let (bold, green, reset) = if use_color {
        ("\x1b[1m", "\x1b[32m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    println!("{bold}Registered Search Engines ({}):{reset}\n", infos.len());
    for e in infos {
        let cats: Vec<&str> = e.categories.iter().map(|c| c.as_str()).collect();
        println!(
            "  {green}{:<14}{reset} categories=[{}] bangs=[{}] weight={}",
            e.name,
            cats.join(", "),
            e.bangs.join(" "),
            e.weight
        );
        println!("                 {}", e.description);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_clap_cli_configuration() {
        Cli::command().debug_assert();
    }
}
