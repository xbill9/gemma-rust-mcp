//! Demo CLI: the MCP sibling of `gemma-rust`. Instead of calling the model's HTTP endpoint, it
//! launches a rig's own MCP server (`python3 server.py`) over stdio, asks the question through
//! the rig's read-only tools, and prints every step of the MCP exchange.

use std::path::PathBuf;
use std::process::{ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, JsonObject};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const DEFAULT_PROMPT: &str = "In one sentence, what is a TPU?";
const INTERACTIVE_HELP: &str = "Commands: /status re-runs the status tools, /rig-help asks the \
     server for its own help, /help, /quit (or Ctrl-D).";

#[derive(Parser)]
#[command(
    version,
    about = "Ask a Gemma 4 rig one question through its MCP server and show the details"
)]
struct Args {
    /// Prompt to send [default: "In one sentence, what is a TPU?"]; with --interactive, the first turn
    prompt: Option<String>,

    /// Keep asking: read prompts from the terminal, one query tool call each
    #[arg(short, long)]
    interactive: bool,

    /// Only run the rig's read-only status tools (model, GPU memory, deployment); skip the query
    #[arg(long, conflicts_with_all = ["prompt", "interactive", "no_status"])]
    status: bool,

    /// Only run the rig's read-only help tool (its tools and configuration); skip the query
    #[arg(long, conflicts_with_all = ["prompt", "interactive", "status", "no_status"])]
    rig_help: bool,

    /// Which rig's MCP server to launch
    #[arg(short, long, value_enum, env = "GEMMA_RIG", default_value_t = Rig::Local)]
    rig: Rig,

    /// Rig directory containing server.py [default: ~/gemma4-dev/<rig directory>]
    #[arg(long, env = "GEMMA_RIG_DIR")]
    rig_dir: Option<PathBuf>,

    /// Python interpreter that runs server.py
    #[arg(long, env = "GEMMA_PYTHON", default_value = "python3")]
    python: String,

    /// max_tokens for the local rig's query_model (the Cloud Run stats tool takes none)
    #[arg(long, default_value_t = 1024)]
    max_tokens: u32,

    /// Skip the status tools and only run the query
    #[arg(long)]
    no_status: bool,

    /// Also print every tool's input schema
    #[arg(long)]
    schemas: bool,

    /// Timeout in seconds for the handshake and for each tool call
    #[arg(long, default_value_t = 600)]
    timeout: u64,
}

#[derive(Clone, Copy, ValueEnum)]
enum Rig {
    /// llama.cpp on the local GPU: ~/gemma4-dev/local-llamacpp-1650ti-2b-q4_0
    Local,
    /// vLLM on Cloud Run: ~/gemma4-dev/gpu-2B-cloudrun-devops-agent
    Cloudrun,
}

impl Rig {
    fn dir_name(self) -> &'static str {
        match self {
            Rig::Local => "local-llamacpp-1650ti-2b-q4_0",
            Rig::Cloudrun => "gpu-2B-cloudrun-devops-agent",
        }
    }

    /// Read-only tools called before the query. Never add a tool that deploys, destroys,
    /// scales, starts, or stops anything: those are billed or destructive.
    fn status_tools(self) -> &'static [&'static str] {
        match self {
            Rig::Local => &["gpu_status", "model_server_status", "model_info"],
            Rig::Cloudrun => &[
                "cloudrun_status",
                "cloudrun_get_system_status",
                "cloudrun_get_model_details",
            ],
        }
    }

    /// The server's own help text: its tools and, on Cloud Run, its configuration. Read-only.
    fn help_tool(self) -> &'static str {
        match self {
            Rig::Local => "get_help",
            Rig::Cloudrun => "cloudrun_get_help",
        }
    }

    fn query_tool(self) -> &'static str {
        match self {
            Rig::Local => "query_model",
            Rig::Cloudrun => "cloudrun_query_gemma4_with_stats",
        }
    }

    fn query_args(self, prompt: &str, max_tokens: u32) -> JsonObject {
        match self {
            Rig::Local => object(json!({"prompt": prompt, "max_tokens": max_tokens})),
            Rig::Cloudrun => object(json!({"prompt": prompt})),
        }
    }
}

type Client = RunningService<RoleClient, ()>;

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("\nerror: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<ExitCode> {
    let rig = args.rig;
    let dir = match &args.rig_dir {
        Some(d) => d.clone(),
        None => home()?.join("gemma4-dev").join(rig.dir_name()),
    };
    if !dir.join("server.py").is_file() {
        bail!("no server.py in {} (set --rig-dir)", dir.display());
    }
    let timeout = Duration::from_secs(args.timeout);

    section("MCP server");
    field("rig", rig.dir_name());
    field("command", format!("{} server.py", args.python));
    field("working dir", dir.display());
    field("transport", "stdio (child process)");

    // The server runs from its rig directory: the local rig opens server.py and tpu.env by
    // relative path.
    let mut cmd = Command::new(&args.python);
    cmd.arg("server.py").current_dir(&dir);
    let (transport, stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("launching `{} server.py`", args.python))?;
    if let Some(pid) = transport.id() {
        field("pid", pid);
    }

    // Collect the server's own log (stderr) to print at the end.
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut seen = 0;
    let log_reader = stderr.map(|stderr| {
        let log = Arc::clone(&log);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log.lock().unwrap().push(line);
            }
        })
    });

    let started = Instant::now();
    let client = match tokio::time::timeout(timeout, ().serve(transport)).await {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => {
            print_log(&log, &mut seen);
            return Err(e).context("MCP initialize handshake failed");
        }
        Err(_) => {
            print_log(&log, &mut seen);
            bail!("MCP initialize handshake timed out after {}s", args.timeout);
        }
    };
    field("initialize", format!("ok in {}", ms(started.elapsed())));

    if let Some(info) = client.peer_info() {
        section("Server info");
        match &info.server_info {
            Some(server) => {
                field("name", &server.name);
                let version = server.version.as_str();
                field(
                    "version",
                    if version.is_empty() {
                        "(empty)"
                    } else {
                        version
                    },
                );
            }
            None => field("name", "(not reported)"),
        }
        field("protocol", plain(&info.protocol_version));
        let caps = &info.capabilities;
        let offered: Vec<&str> = [
            ("tools", caps.tools.is_some()),
            ("resources", caps.resources.is_some()),
            ("prompts", caps.prompts.is_some()),
            ("logging", caps.logging.is_some()),
            ("completions", caps.completions.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect();
        field("capabilities", offered.join(", "));
        if let Some(instructions) = info
            .instructions
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            field("instructions", first_line(instructions));
        }
    }

    let status_tools: &[&str] = if args.no_status || args.rig_help {
        &[]
    } else {
        rig.status_tools()
    };
    // The tools this run calls before any interactive commands.
    let mut calls = status_tools.to_vec();
    if args.rig_help {
        calls.push(rig.help_tool());
    } else if !args.status {
        calls.push(rig.query_tool());
    }

    section("Tools");
    let started = Instant::now();
    let tools = client.list_all_tools().await.context("tools/list")?;
    field(
        "tools/list",
        format!("{} tools in {}", tools.len(), ms(started.elapsed())),
    );
    let width = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for tool in &tools {
        let called = calls.contains(&tool.name.as_ref());
        let description = tool
            .description
            .as_deref()
            .map(first_line)
            .unwrap_or_default();
        println!(
            "  {} {:<width$}  {}",
            if called { "*" } else { " " },
            tool.name,
            truncate(description, 60)
        );
        if args.schemas {
            println!(
                "{}",
                indent(&serde_json::to_string_pretty(&*tool.input_schema)?, 6)
            );
        }
    }
    println!("  (* = called by this demo, which only calls read-only tools)");
    for name in &calls {
        if !tools.iter().any(|t| t.name == *name) {
            bail!("the server does not offer the `{name}` tool");
        }
    }

    let mut status_ok = true;
    for name in status_tools {
        status_ok &= call(&client, name, JsonObject::new(), timeout)
            .await?
            .is_some();
    }
    let answered = if args.rig_help {
        call(&client, rig.help_tool(), JsonObject::new(), timeout)
            .await?
            .is_some()
    } else if args.status {
        status_ok
    } else if args.interactive {
        interactive(&client, &args, timeout, &log, &mut seen).await?;
        true
    } else {
        let prompt = args.prompt.as_deref().unwrap_or(DEFAULT_PROMPT);
        ask(&client, &args, prompt, timeout).await?
    };

    client
        .cancel()
        .await
        .context("shutting down the MCP session")?;
    if let Some(reader) = log_reader {
        let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
    }
    print_log(&log, &mut seen);

    Ok(if answered {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

/// Ask one question through the rig's query tool. True when the model answered.
async fn ask(client: &Client, args: &Args, prompt: &str, timeout: Duration) -> Result<bool> {
    let rig = args.rig;
    let arguments = rig.query_args(prompt, args.max_tokens);
    // query_model answers "📡 Reasoning only — no answer yet" when Gemma 4 runs out of
    // max_tokens mid-thought: not a failure, but not an answer either.
    Ok(
        match call(client, rig.query_tool(), arguments, timeout).await? {
            Some(text) if text.trim_start().starts_with('📡') => {
                println!(
                    "  No answer: the model was still reasoning when it stopped. Raise --max-tokens."
                );
                false
            }
            Some(_) => true,
            None => false,
        },
    )
}

/// Read prompts until /quit or Ctrl-D, one query tool call each, printing the server's new log
/// lines after every turn.
async fn interactive(
    client: &Client,
    args: &Args,
    timeout: Duration,
    log: &Mutex<Vec<String>>,
    seen: &mut usize,
) -> Result<()> {
    print_fresh_log(log, seen).await;
    section("Interactive");
    println!(
        "  Each prompt is one `{}` call. The tool takes a single prompt, so the model does not see \
         earlier turns.",
        args.rig.query_tool()
    );
    println!("  {INTERACTIVE_HELP}");
    let mut editor = DefaultEditor::new().context("starting the line editor")?;
    let mut next = args.prompt.clone();
    loop {
        let line = match next.take() {
            Some(prompt) => prompt,
            // readline blocks; block_in_place keeps the MCP transport's tasks running meanwhile.
            None => match tokio::task::block_in_place(|| editor.readline("\ngemma-mcp> ")) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => break,
                Err(e) => return Err(e).context("reading a prompt"),
            },
        };
        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(prompt);
        let result: Result<()> = match prompt {
            "/quit" | "/exit" => break,
            "/help" => {
                println!("  {INTERACTIVE_HELP}");
                continue;
            }
            "/status" => {
                async {
                    for name in args.rig.status_tools() {
                        call(client, name, JsonObject::new(), timeout).await?;
                    }
                    Ok(())
                }
                .await
            }
            "/rig-help" => call(client, args.rig.help_tool(), JsonObject::new(), timeout)
                .await
                .map(|_| ()),
            p if p.starts_with('/') => {
                println!("  Unknown command {p}. {INTERACTIVE_HELP}");
                continue;
            }
            _ => ask(client, args, prompt, timeout).await.map(|_| ()),
        };
        if let Err(e) = result {
            eprintln!("\nerror: {e:#}");
        }
        print_fresh_log(log, seen).await;
    }
    Ok(())
}

/// Give the stderr reader a moment to catch up, then print what the server logged.
async fn print_fresh_log(log: &Mutex<Vec<String>>, seen: &mut usize) {
    tokio::time::sleep(Duration::from_millis(100)).await;
    print_log(log, seen);
}

/// Call one tool and print the exchange. Returns the result text, or None if the tool failed.
async fn call(
    client: &Client,
    name: &'static str,
    arguments: JsonObject,
    timeout: Duration,
) -> Result<Option<String>> {
    section(&format!("tools/call {name}"));
    field("arguments", Value::Object(arguments.clone()));
    let params = CallToolRequestParams::new(name).with_arguments(arguments);
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, client.call_tool(params))
        .await
        .with_context(|| format!("tools/call {name} timed out"))?
        .with_context(|| format!("tools/call {name}"))?;
    field("latency", ms(started.elapsed()));
    let is_error = result.is_error.unwrap_or(false);
    field("isError", is_error);

    let text = text_of(&result);
    // These rigs report failure in the text (a leading ❌) and leave isError false.
    let reported_failure = text.trim_start().starts_with('❌');
    if reported_failure && !is_error {
        field(
            "note",
            "the tool reported a failure in its text; MCP isError is false",
        );
    }
    // FastMCP mirrors a string return as {"result": <the same text>}; don't print it twice.
    match &result.structured_content {
        Some(s) if s.get("result").and_then(Value::as_str) == Some(text.as_str()) => field(
            "structuredContent",
            "{\"result\": …} (same text as the content below)",
        ),
        Some(s) => field("structuredContent", s),
        None => {}
    }
    println!("  result:");
    println!("{}", indent(text.trim_end(), 4));
    Ok((!is_error && !reported_failure).then_some(text))
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .map(|block| match block.as_text() {
            Some(t) => t.text.clone(),
            None => {
                let kind = serde_json::to_value(block)
                    .ok()
                    .and_then(|v| v["type"].as_str().map(String::from))
                    .unwrap_or_else(|| "non-text".to_string());
                format!("[{kind} content]")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print the server's stderr lines that have not been printed yet.
fn print_log(log: &Mutex<Vec<String>>, seen: &mut usize) {
    section("Server log (stderr)");
    let lines = log.lock().unwrap();
    let fresh = &lines[(*seen).min(lines.len())..];
    if fresh.is_empty() {
        println!("  (nothing)");
    }
    for line in fresh {
        println!("  {line}");
    }
    *seen = lines.len();
}

fn object(value: Value) -> JsonObject {
    match value {
        Value::Object(map) => map,
        _ => JsonObject::new(),
    }
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; pass --rig-dir")
}

/// A serializable value as plain text (strings without their quotes).
fn plain(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => "?".to_string(),
    }
}

fn section(title: &str) {
    println!(
        "\n== {title} {}",
        "=".repeat(66usize.saturating_sub(title.chars().count()))
    );
}

fn field(key: &str, value: impl std::fmt::Display) {
    println!("  {key:<22} {value}");
}

fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1000.0)
}

fn first_line(text: &str) -> &str {
    text.trim().lines().next().unwrap_or("").trim()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    }
}

fn indent(text: &str, width: usize) -> String {
    let pad = " ".repeat(width);
    text.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
