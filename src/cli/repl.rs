use std::io::{self, BufRead, Write};

use crate::cli::{render_search_response, CliContext, OutputFormat};
use crate::models::{Category, SearchQuery, SearchResponse};

/// Interactive terminal search REPL (`rseek repl`).
pub async fn run_repl(
    ctx: &CliContext,
    initial_category: Category,
    initial_engines: Vec<String>,
    initial_lang: String,
    initial_limit: usize,
    no_color: bool,
) -> Result<(), String> {
    let mut category = initial_category;
    let mut engines = initial_engines;
    let mut lang = initial_lang;
    let mut limit = initial_limit;
    let mut page: u32 = 1;
    let mut last_query: Option<String> = None;
    let mut last_response: Option<SearchResponse> = None;

    println!("rseek interactive mode — type a query or :help (:quit to exit)");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("\nrseek [{category}]> ");
        stdout.flush().map_err(|e| format!("stdout error: {e}"))?;

        let mut line = String::new();
        let bytes = stdin
            .lock()
            .read_line(&mut line)
            .map_err(|e| format!("stdin error: {e}"))?;
        if bytes == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(cmd_line) = trimmed.strip_prefix(':') {
            let mut parts = cmd_line.split_whitespace();
            let cmd = parts.next().unwrap_or("");
            let arg = parts.collect::<Vec<_>>().join(" ");

            match cmd {
                "q" | "quit" | "exit" => break,
                "help" | "h" => print_repl_help(),
                "cat" | "category" => match arg.parse::<Category>() {
                    Ok(c) => {
                        category = c;
                        page = 1;
                        println!("Category set to '{category}'.");
                    }
                    Err(e) => eprintln!("{e}"),
                },
                "engines" | "engine" => {
                    if arg.is_empty() || arg == "auto" {
                        engines.clear();
                        println!("Engines reset to category auto-selection.");
                    } else {
                        engines = arg
                            .split(',')
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                        println!("Engines set to: {}", engines.join(", "));
                    }
                }
                "lang" => {
                    if !arg.is_empty() {
                        lang = arg;
                        println!("Language set to '{lang}'.");
                    }
                }
                "limit" => match arg.parse::<usize>() {
                    Ok(n) if (1..=50).contains(&n) => {
                        limit = n;
                        println!("Limit set to {limit}.");
                    }
                    _ => eprintln!("Usage: :limit <1..50>"),
                },
                "page" => match arg.parse::<u32>() {
                    Ok(p) if p >= 1 => {
                        page = p;
                        if let Some(q) = &last_query {
                            let req = build_req(q, category, &engines, &lang, page, limit);
                            if let Some(resp) =
                                execute_and_print(ctx, req, OutputFormat::Pretty, no_color).await
                            {
                                last_response = Some(resp);
                            }
                        }
                    }
                    _ => eprintln!("Usage: :page <1..>"),
                },
                "next" | "n" => {
                    if let Some(q) = &last_query {
                        page += 1;
                        let req = build_req(q, category, &engines, &lang, page, limit);
                        if let Some(resp) =
                            execute_and_print(ctx, req, OutputFormat::Pretty, no_color).await
                        {
                            last_response = Some(resp);
                        }
                    } else {
                        eprintln!("No previous query.");
                    }
                }
                "prev" | "p" => {
                    if let Some(q) = &last_query {
                        if page > 1 {
                            page -= 1;
                        }
                        let req = build_req(q, category, &engines, &lang, page, limit);
                        if let Some(resp) =
                            execute_and_print(ctx, req, OutputFormat::Pretty, no_color).await
                        {
                            last_response = Some(resp);
                        }
                    } else {
                        eprintln!("No previous query.");
                    }
                }
                "json" => {
                    if let Some(resp) = &last_response {
                        render_search_response(resp, OutputFormat::Json, true);
                    } else {
                        eprintln!("No previous search response.");
                    }
                }
                "info" => {
                    if let (Ok(idx), Some(resp)) = (arg.parse::<usize>(), &last_response) {
                        if let Some(item) = idx.checked_sub(1).and_then(|i| resp.results.get(i)) {
                            let pretty = serde_json::to_string_pretty(item).unwrap_or_default();
                            println!("{pretty}");
                        } else {
                            eprintln!("Result #{idx} out of range.");
                        }
                    } else {
                        eprintln!("Usage: :info <result-number>");
                    }
                }
                _ => eprintln!("Unknown command ':{cmd}'. Type :help for commands."),
            }
            continue;
        }

        page = 1;
        last_query = Some(trimmed.to_string());
        let req = build_req(trimmed, category, &engines, &lang, page, limit);
        if let Some(resp) = execute_and_print(ctx, req, OutputFormat::Pretty, no_color).await {
            last_response = Some(resp);
        }
    }

    Ok(())
}

fn build_req(
    query: &str,
    category: Category,
    engines: &[String],
    lang: &str,
    page: u32,
    limit: usize,
) -> SearchQuery {
    SearchQuery {
        query: query.to_string(),
        category,
        engines: engines.to_vec(),
        lang: lang.to_string(),
        page,
        limit,
        timeout_ms: 0,
        no_cache: false,
    }
}

async fn execute_and_print(
    ctx: &CliContext,
    req: SearchQuery,
    format: OutputFormat,
    no_color: bool,
) -> Option<SearchResponse> {
    match ctx.search(req).await {
        Ok(resp) => {
            render_search_response(&resp, format, no_color);
            Some(resp)
        }
        Err(err) => {
            eprintln!("Error: {err}");
            None
        }
    }
}

fn print_repl_help() {
    println!(
        "\
REPL commands:
  <query>              Search immediately (supports !w, !gh, !so, !arxiv, !crates, !npm, !hn, !img)
  :cat <category>      Switch category (general, code, news, science, packages, wiki, images, all)
  :engines <list|auto> Pin comma-separated engines or reset to auto
  :lang <code>         Set language code (e.g. en, de, fr, ar)
  :limit <n>           Set max displayed results (1..50)
  :next / :prev        Go to next / previous results page
  :page <n>            Jump to specific page
  :info <n>            Print full JSON metadata for result #n
  :json                Print last full response as JSON
  :quit                Exit REPL"
    );
}
