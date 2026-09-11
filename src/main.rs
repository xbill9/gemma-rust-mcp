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
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Parser)]
#[command(
    version,
    about = "Ask a Gemma 4 rig one question through its MCP server and show the details"
)]
struct Args {
    /// Prompt to send
    #[arg(default_value = "In one sentence, what is a TPU?")]
    prompt: String,

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
            Rig::Cloudrun => &["cloudrun_get_system_status", "cloudrun_get_model_details"],
        }
    }

    fn query(self, args: &Args) -> (&'static str, JsonObject) {
        match self {
            Rig::Local => (
                "query_model",
                object(json!({"prompt": args.prompt, "max_tokens": args.max_tokens})),
            ),
            Rig::Cloudrun => (
                "cloudrun_query_gemma4_with_stats",
                object(json!({"prompt": args.prompt})),
            ),
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
            print_log(&log);
            return Err(e).context("MCP initialize handshake failed");
        }
        Err(_) => {
            print_log(&log);
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

    let status_tools: &[&str] = if args.no_status {
        &[]
    } else {
        rig.status_tools()
    };
    let (query_tool, query_args) = rig.query(&args);

    section("Tools");
    let started = Instant::now();
    let tools = client.list_all_tools().await.context("tools/list")?;
    field(
        "tools/list",
        format!("{} tools in {}", tools.len(), ms(started.elapsed())),
    );
    let width = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for tool in &tools {
        let called = tool.name == query_tool || status_tools.contains(&tool.name.as_ref());
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
    for name in status_tools.iter().chain([&query_tool]) {
        if !tools.iter().any(|t| t.name == *name) {
            bail!("the server does not offer the `{name}` tool");
        }
    }

    for name in status_tools {
        call(&client, name, JsonObject::new(), timeout).await?;
    }
    // query_model answers "📡 Reasoning only — no answer yet" when Gemma 4 runs out of
    // max_tokens mid-thought: not a failure, but not an answer either.
    let answered = match call(&client, query_tool, query_args, timeout).await? {
        Some(text) if text.trim_start().starts_with('📡') => {
            println!(
                "  No answer: the model was still reasoning when it stopped. Raise --max-tokens."
            );
            false
        }
        Some(_) => true,
        None => false,
    };

    client
        .cancel()
        .await
        .context("shutting down the MCP session")?;
    if let Some(reader) = log_reader {
        let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
    }
    print_log(&log);

    Ok(if answered {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
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

fn print_log(log: &Mutex<Vec<String>>) {
    section("Server log (stderr)");
    let lines = log.lock().unwrap();
    if lines.is_empty() {
        println!("  (nothing)");
    }
    for line in lines.iter() {
        println!("  {line}");
    }
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
