//! `polymarket strategy` — manage and run the local autonomous strategy
//! engine. Most users will drive this from the TUI (`polymarket tui`), but
//! every action is available headless here so strategies can run on a server
//! or under a process manager with no UI attached.

use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::output::OutputFormat;
use crate::paper::store;
use crate::strategy::engine::{ExecutionMode, StrategyEngine};
use crate::strategy::registry;

#[derive(Args)]
pub struct StrategyArgs {
    #[command(subcommand)]
    pub command: StrategyCommand,
}

#[derive(Subcommand)]
pub enum StrategyCommand {
    /// List available strategy plugins and your configured roster
    List,

    /// Add a strategy instance to your roster
    Add {
        /// Strategy kind (see `strategy list`), e.g. momentum
        kind: String,
        /// Instance id (defaults to the kind)
        #[arg(long)]
        id: Option<String>,
        /// Comma-separated token IDs to watch/trade
        #[arg(long)]
        tokens: String,
        /// Strategy parameters as JSON (see `strategy list` for the shape).
        /// Omitted fields fall back to the kind's defaults.
        #[arg(long)]
        params: Option<String>,
    },

    /// Edit an existing strategy's tokens and/or parameters
    Edit {
        /// Instance id
        id: String,
        /// New comma-separated token IDs (keeps the current set if omitted)
        #[arg(long)]
        tokens: Option<String>,
        /// New parameters as JSON (merged is not done — pass the full object)
        #[arg(long)]
        params: Option<String>,
    },

    /// Remove a strategy instance from your roster
    Remove {
        /// Instance id
        id: String,
    },

    /// Enable a strategy (allowed to run)
    Enable {
        /// Instance id
        id: String,
    },

    /// Disable a strategy
    Disable {
        /// Instance id
        id: String,
    },

    /// Show the roster and runtime status
    Status,

    /// Run the engine loop in the foreground (Ctrl-C to stop)
    Run {
        /// Only run this instance (default: all enabled)
        #[arg(long)]
        id: Option<String>,
        /// Seconds between ticks
        #[arg(long, default_value = "10")]
        interval: u64,
        /// Route orders to the live CLOB instead of the paper account.
        /// Real funds — needs a configured wallet.
        #[arg(long)]
        live: bool,
    },

    /// Print the strategy log file
    Logs {
        /// Max lines to show (most recent)
        #[arg(long, default_value = "50")]
        limit: usize,
    },
}

pub async fn execute(args: StrategyArgs, output: OutputFormat) -> Result<()> {
    match args.command {
        StrategyCommand::List => list(output),
        StrategyCommand::Add {
            kind,
            id,
            tokens,
            params,
        } => {
            let id = id.unwrap_or_else(|| kind.clone());
            let tokens = parse_tokens(&tokens)?;
            let params = parse_params(params.as_deref())?;
            let engine = build_engine(ExecutionMode::Paper)?;
            engine.add_with_params(&id, &kind, tokens, params)?;
            println!("Added strategy '{id}' ({kind}). Enable + run with `strategy run`.");
            Ok(())
        }
        StrategyCommand::Edit { id, tokens, params } => {
            let engine = build_engine(ExecutionMode::Paper)?;
            let current = engine
                .snapshot()
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| anyhow::anyhow!("No strategy with id '{id}'"))?;
            let tokens = match tokens {
                Some(t) => parse_tokens(&t)?,
                None => current.tokens.clone(),
            };
            let params = match params {
                Some(p) => parse_params(Some(&p))?,
                None => current.params.clone(),
            };
            engine.update(&id, tokens, params)?;
            println!("Updated strategy '{id}'.");
            Ok(())
        }
        StrategyCommand::Remove { id } => {
            let engine = build_engine(ExecutionMode::Paper)?;
            engine.remove(&id)?;
            println!("Removed strategy '{id}'.");
            Ok(())
        }
        StrategyCommand::Enable { id } => {
            let engine = build_engine(ExecutionMode::Paper)?;
            engine.set_enabled(&id, true)?;
            println!("Enabled '{id}'.");
            Ok(())
        }
        StrategyCommand::Disable { id } => {
            let engine = build_engine(ExecutionMode::Paper)?;
            engine.set_enabled(&id, false)?;
            println!("Disabled '{id}'.");
            Ok(())
        }
        StrategyCommand::Status => {
            let engine = build_engine(ExecutionMode::Paper)?;
            print_status(&engine, output);
            Ok(())
        }
        StrategyCommand::Run { id, interval, live } => run(id, interval, live).await,
        StrategyCommand::Logs { limit } => {
            print_log_file(limit);
            Ok(())
        }
    }
}

fn list(output: OutputFormat) -> Result<()> {
    let available = registry::available();
    let book = crate::strategy::config::load().unwrap_or_default();
    match output {
        OutputFormat::Json => {
            let plugins: Vec<_> = available
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "kind": m.kind,
                        "summary": m.summary,
                        "default_params": m.default_params,
                    })
                })
                .collect();
            crate::output::print_json(&serde_json::json!({
                "available": plugins,
                "roster": book.strategies,
            }))?;
        }
        OutputFormat::Table => {
            println!("Available strategy plugins:");
            for m in &available {
                println!("  {:<16} {}", m.kind, m.summary);
            }
            println!();
            if book.strategies.is_empty() {
                println!("No strategies configured. Add one:");
                println!("  polymarket strategy add momentum --tokens <TOKEN_ID>");
            } else {
                println!("Your roster:");
                for c in &book.strategies {
                    println!(
                        "  {:<16} {:<14} {} token(s){}",
                        c.id,
                        c.kind,
                        c.tokens.len(),
                        if c.enabled { " [enabled]" } else { "" }
                    );
                }
            }
        }
    }
    Ok(())
}

fn print_status(engine: &StrategyEngine, output: OutputFormat) {
    let snap = engine.snapshot();
    match output {
        OutputFormat::Json => {
            let rows: Vec<_> = snap
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "kind": s.kind,
                        "enabled": s.enabled,
                        "running": s.running,
                        "tokens": s.tokens,
                        "signals": s.signals,
                        "orders": s.orders,
                        "errors": s.errors,
                        "last_action": s.last_action,
                    })
                })
                .collect();
            let _ = crate::output::print_json(&serde_json::json!({ "strategies": rows }));
        }
        OutputFormat::Table => {
            if snap.is_empty() {
                println!("No strategies configured.");
                return;
            }
            println!(
                "{:<14} {:<14} {:<9} {:<7} {:<8} LAST ACTION",
                "ID", "KIND", "STATE", "ORDERS", "SIGNALS"
            );
            for s in &snap {
                let state = if s.running {
                    "running"
                } else if s.enabled {
                    "idle"
                } else {
                    "disabled"
                };
                println!(
                    "{:<14} {:<14} {:<9} {:<7} {:<8} {}",
                    s.id,
                    s.kind,
                    state,
                    s.orders,
                    s.signals,
                    s.last_action.as_deref().unwrap_or("-")
                );
            }
        }
    }
}

async fn run(id: Option<String>, interval: u64, live: bool) -> Result<()> {
    if store::load()?.is_none() {
        bail!(
            "No paper account. Run `polymarket paper enable` first (the engine trades the paper account)."
        );
    }
    let mode = if live {
        ExecutionMode::Live
    } else {
        ExecutionMode::Paper
    };
    let engine = build_engine_with_interval(mode, interval)?;

    match &id {
        Some(id) => {
            engine.set_enabled(id, true)?;
            engine.start(id)?;
        }
        None => engine.start_all(),
    }

    if engine.running_count() == 0 {
        bail!("No strategies running. Add and enable one with `strategy add` / `strategy enable`.");
    }

    println!(
        "Strategy engine running ({mode} mode, {interval}s tick). {} strategy(ies) active. Ctrl-C to stop.\n",
        engine.running_count()
    );

    let mut seen = 0usize;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nStopping engine. Account state saved.");
                let _ = engine.save_account();
                break;
            }
            r = engine.tick() => {
                if let Err(e) = r {
                    eprintln!("tick error: {e}");
                }
                seen = drain_logs(&engine, seen);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    Ok(())
}

/// Print any log lines we haven't shown yet; returns the new seen count.
fn drain_logs(engine: &StrategyEngine, seen: usize) -> usize {
    let logs = engine.recent_logs(500);
    if logs.len() > seen {
        for line in &logs[seen..] {
            println!(
                "{} [{}] {} — {}",
                line.time.format("%H:%M:%S"),
                line.level.label(),
                line.source,
                line.message
            );
        }
    }
    logs.len()
}

fn parse_tokens(s: &str) -> Result<Vec<String>> {
    let tokens: Vec<String> = s
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if tokens.is_empty() {
        bail!("Provide at least one token via --tokens");
    }
    Ok(tokens)
}

/// Parse a JSON params string, or `Null` (kind defaults) when not given.
fn parse_params(s: Option<&str>) -> Result<serde_json::Value> {
    match s {
        None => Ok(serde_json::Value::Null),
        Some(raw) => {
            serde_json::from_str(raw).map_err(|e| anyhow::anyhow!("Invalid --params JSON: {e}"))
        }
    }
}

fn build_engine(mode: ExecutionMode) -> Result<StrategyEngine> {
    build_engine_with_interval(mode, 10)
}

fn build_engine_with_interval(mode: ExecutionMode, interval: u64) -> Result<StrategyEngine> {
    let account = store::load()?.unwrap_or_else(|| {
        crate::paper::types::PaperAccount::new(
            crate::paper::types::default_starting_balance(),
            false,
        )
    });
    let account = Arc::new(Mutex::new(account));
    Ok(StrategyEngine::new(account, interval, mode))
}

fn print_log_file(limit: usize) {
    let Ok(dir) = crate::config::config_dir() else {
        return;
    };
    let path = dir.join("strategy.log");
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(limit);
            for line in &lines[start..] {
                println!("{line}");
            }
        }
        Err(_) => println!("No strategy log yet at {}.", path.display()),
    }
}
