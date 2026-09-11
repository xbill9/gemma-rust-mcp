# gemma-rust-mcp

The MCP sibling of [gemma-rust](https://github.com/xbill9/gemma-rust). Both are demo CLIs that
ask Gemma 4 E2B one question and print everything about the exchange. gemma-rust calls the
model's HTTP endpoint directly. **This one is an MCP client**: it launches a rig's own MCP server
(`python3 server.py`) as a child process over stdio, and asks through the rig's tools.

It works with two rigs:

- **local**: llama.cpp on a laptop GPU (GTX 1650 Ti)
- **cloudrun**: vLLM on an NVIDIA L4 on Cloud Run

Built with [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk), the official Rust MCP SDK.

## What it prints

Every run shows each step of the MCP session, in labelled sections:

- the server command, working directory, pid, and `initialize` handshake latency
- the server's name, version, MCP protocol version, and capabilities
- every tool the server offers, with the ones this demo calls marked `*` (`--schemas` adds each
  tool's input schema)
- each `tools/call`: arguments, latency, `isError`, and the returned text
- the MCP server's own stderr log

## It only calls read-only tools

The rig servers also expose tools that deploy, destroy, scale, start, and stop things. Those are
billed or destructive, so this CLI calls a fixed list of read-only tools and has no option to call
an arbitrary one.

| rig | status tools | help tool | query tool |
|---|---|---|---|
| `local` | `gpu_status`, `model_server_status`, `model_info` | `get_help` | `query_model(prompt, max_tokens)` |
| `cloudrun` | `cloudrun_status`, `cloudrun_get_system_status`, `cloudrun_get_model_details` | `cloudrun_get_help` | `cloudrun_query_gemma4_with_stats(prompt)` |

`--status` (`make status`, `make status-cloud`) runs only the status tools: on the local rig the
GPU and its memory, the model server, and the checkpoint; on Cloud Run the deployment's
conditions and revision, the system dashboard, and the served model. The rig tools report no
Cloud Run CPU/memory limits; gemma-rust's `make status-cloud` shows those.

`--rig-help` (`make rig-help`, `make rig-help-cloud`) runs only the rig's help tool, which is
the server describing itself. The local rig lists its tools. The Cloud Run rig also prints its
configuration, including the GCP project and region, so redact those before you share the output.
`--help` is this CLI's own help.

## Build

Requires Rust 1.88+ (for `rmcp`), and Python 3 with `mcp>=2` to run the rig servers.

```sh
cargo build --release
./target/release/gemma-rust-mcp --help
```

## Usage

```sh
gemma-rust-mcp "In one sentence, what is a TPU?"                  # local rig
gemma-rust-mcp --rig cloudrun "In one sentence, what is a TPU?"   # Cloud Run rig
gemma-rust-mcp --no-status --schemas "Why is the sky blue?"
gemma-rust-mcp -i                                                 # interactive (make chat / chat-cloud)
```

| Option | Env | Default | |
|---|---|---|---|
| `-r, --rig` | `GEMMA_RIG` | `local` | `local` or `cloudrun` |
| `--rig-dir` | `GEMMA_RIG_DIR` | `~/gemma4-dev/<rig directory>` | Directory containing the rig's `server.py` |
| `--python` | `GEMMA_PYTHON` | `python3` | Interpreter that runs `server.py` |
| `--max-tokens` | | `1024` | Passed to the local `query_model`; the Cloud Run tool takes none |
| `--no-status` | | | Skip the status tools, only run the query |
| `--schemas` | | | Print every tool's input schema |
| `--timeout` | | `600` | Seconds, for the handshake and for each tool call |
| `-i, --interactive` | | | Keep the MCP session open and read prompts until `/quit` or Ctrl-D |
| `--status` | | | Only run the status tools; skip the query |
| `--rig-help` | | | Only run the rig's help tool; skip the query |
| `-h, --help` | | | This CLI's options |

Exit codes: `0` answered, `1` error, `2` the query tool failed or returned no answer (with
`--status` or `--rig-help`: the tool reported a failure).

**Interactive mode** launches the server, lists its tools, and runs the status tools once. After
that, each prompt is one query tool call, followed by whatever the server logged during that call.
The rig tools take a single prompt, so **the model does not see earlier turns** (gemma-rust `-i`
does keep the conversation). `/status` re-runs the status tools, `/rig-help` calls the rig's help
tool, and `/help` lists the commands. A prompt given on the command
line becomes the first turn. A failed turn is printed and the session continues; it exits `0` on
`/quit` or Ctrl-D.

The rig servers are not part of this repo. Any MCP server started as `server.py` that offers the
tools in the table above works; point `--rig-dir` at it. The Cloud Run server finds its service
URL and project on its own (with `gcloud`), so nothing about the deployment is configured here.

## MCP vs. calling the endpoint

The two siblings ask the same model the same question and see different things:

| | gemma-rust (HTTP) | gemma-rust-mcp (MCP) |
|---|---|---|
| what you get | the raw OpenAI-style response | whatever the tool chooses to report, as markdown |
| reasoning | full text | local: suppressed, only its length; Cloud Run: none |
| token counts | exact, from `usage` | local: exact; Cloud Run: approximate (counts stream chunks) |
| server timings | llama.cpp's `timings` object | tokens/s, and TTFT on Cloud Run |
| extra | nothing | GPU, model, and service status from the rig's own tools |
| moving parts | one HTTP call | a Python server that makes the same HTTP call |

**Failures come back as text, not as protocol errors.** The rig tools return markdown that starts
with `❌` and leave MCP's `isError` false. The CLI treats a leading `❌` as a failure and says so.
When Gemma 4 runs out of `max_tokens` while still thinking, the local `query_model` returns
"📡 Reasoning only — no answer yet"; the CLI reports that as no answer and exits `2`.

**Known issue in the Cloud Run rig:** its streaming query tool leaks Gemma's end-of-turn marker,
so answers end with `<turn|>` (visible in the sample below). That is a bug in the rig's
`server.py`, not in this client, which prints what the tool returns.

## Sample output

Measured 2026-09-11 with the same prompt against both rigs. The Cloud Run service URL is replaced
with a placeholder.

### Local: llama.cpp on a GTX 1650 Ti

```text
== MCP server ========================================================
  rig                    local-llamacpp-1650ti-2b-q4_0
  command                python3 server.py
  working dir            /home/xbill/gemma4-dev/local-llamacpp-1650ti-2b-q4_0
  transport              stdio (child process)
  pid                    278616
  initialize             ok in 859 ms

== Server info =======================================================
  name                   local-llamacpp-1650ti-2b-q4_0
  version                (empty)
  protocol               2025-11-25
  capabilities           tools, resources, prompts

== Tools =============================================================
  tools/list             7 tools in 1 ms
  * gpu_status           Report the local GPU: name, compute capability, VRAM total/…
  * model_info           Report the configured checkpoint, where it is, and the resi…
    start_model_server   Start llama-server on the local GPU. No-op if it is already…
    stop_model_server    Stop the running llama-server. Teardown is complete — nothi…
  * model_server_status  Check whether llama-server is up and serving at the known l…
  * query_model          Send a chat completion to the local endpoint and return the…
    get_help             List the tools this rig exposes.
  (* = called by this demo, which only calls read-only tools)

== tools/call gpu_status =============================================
  arguments              {}
  latency                25 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    📡 **GPU** — `local-llamacpp-1650ti-2b-q4_0`
    
    ```
    NVIDIA GeForce GTX 1650 Ti with Max-Q Design, 7.5, 4096 MiB, 1632 MiB, 2101 MiB, 615.71.09
    ```
    
    ⚠️  GTX 16-series (TU116/TU117): compute capability 7.5 but **no tensor cores**. Do not compare throughput against the T4-based `g4dn`/`g5g` rigs on the strength of a matching compute capability.

== tools/call model_server_status ====================================
  arguments              {}
  latency                29 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    ✅ Serving at http://127.0.0.1:8080 (pid 83619). `/health` → 200.

== tools/call model_info =============================================
  arguments              {}
  latency                1 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    📡 **Model** — `local-llamacpp-1650ti-2b-q4_0`
    
    - **Name:** `google/gemma-4-E2B-it-qat-q4_0-gguf`
    - **Path:** `/home/xbill/models/gemma-4-E2B-it-qat-q4_0/gemma-4-E2B_q4_0-it.gguf`
    - **On disk:** 3.35 GB
    - **Quantization slot:** `q4_0` — but the dominant tensor type is **Q6_K**. Both embedding tensors are Q6_K (2.257 GB of 3.334 GB); only the ~1.08 GB transformer body is actually Q4_0.
    - **Resident on GPU:** ~1.31 GiB. `per_layer_token_embd` (1.93 GB, 58% of the file) is `TENSOR_READ_LAZY` and is served by GET_ROWS out of the mmap.
    
    Run `inspect_gguf.py` to re-derive the split from the artifact rather than trusting these numbers.

== tools/call query_model ============================================
  arguments              {"max_tokens":1024,"prompt":"In one sentence, what is a TPU?"}
  latency                5732 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    ✅ **Reply**
    
    A TPU (Tensor Processing Unit) is a specialized hardware accelerator designed by Google specifically to speed up the computationally intensive matrix operations required for machine learning and deep learning workloads.
    
    ---
    _(plus 1477 chars of reasoning, suppressed)_
    prompt 25 tok · completion 394 tok · 69.5 tok/s

== Server log (stderr) ===============================================
  2026-09-11 11:41:23,179 INFO HTTP Request: GET http://127.0.0.1:8080/health "HTTP/1.1 200 OK"
  2026-09-11 11:41:28,912 INFO HTTP Request: POST http://127.0.0.1:8080/v1/chat/completions "HTTP/1.1 200 OK"
```

### Cloud Run: vLLM on an NVIDIA L4

```text
== MCP server ========================================================
  rig                    gpu-2B-cloudrun-devops-agent
  command                python3 server.py
  working dir            /home/xbill/gemma4-dev/gpu-2B-cloudrun-devops-agent
  transport              stdio (child process)
  pid                    278778
  initialize             ok in 3219 ms

== Server info =======================================================
  name                   Self-Hosted vLLM DevOps Agent
  version                (empty)
  protocol               2025-11-25
  capabilities           tools, resources, prompts

== Tools =============================================================
  tools/list             27 tools in 2 ms
    cloudrun_save_hf_token                            Securely saves a Hugging Face API token to GCP Secret Manag…
    cloudrun_get_endpoint_url                         Returns the current active vLLM endpoint URL without checki…
    cloudrun_list_vertex_models                       Uses the Vertex AI SDK (part of ADK ecosystem) to list mode…
    cloudrun_list_bucket_models                       Lists the contents of the GCS bucket to check for uploaded …
    cloudrun_analyze_cloud_logging                    Fetches and summarizes error logs from Google Cloud Logging…
    cloudrun_suggest_sre_remediation                  Proposes remediation steps for a specific SRE incident usin…
    cloudrun_query                                    Directly queries the self-hosted vLLM model with a custom p…
    cloudrun_get_deployment_config                    Generates the gcloud command to deploy vLLM to Cloud Run wi…
    cloudrun_deploy                                   Deploys vLLM to Cloud Run with GPU.
    cloudrun_destroy                                  Destroys the Cloud Run vLLM service.
    cloudrun_status                                   Checks the status of the Cloud Run vLLM service.
    cloudrun_update_scaling                           Updates the scaling configuration (min and max instances) f…
    cloudrun_get_gpu_deployment_config                Generates a GKE (not Cloud Run) manifest and setup instruct…
    cloudrun_get_vertex_ai_model_copy_instructions    Provides instructions and commands to transfer Gemma model …
    cloudrun_get_huggingfacehub_download_path         Returns the local cache path for a Hugging Face model using…
    cloudrun_get_huggingface_model_copy_instructions  Provides instructions and commands to transfer Gemma model …
    cloudrun_check_gpu_quotas                         Checks GPU quotas for a specific region using gcloud comput…
    cloudrun_verify_model_health                      Runs a deep health check with latency reporting on the Clou…
    cloudrun_query_gemma4                             Queries the self-hosted Gemma 4 model on Cloud Run.
  * cloudrun_query_gemma4_with_stats                  Queries the self-hosted Gemma 4 model on Cloud Run and retu…
  * cloudrun_get_model_details                        Retrieves detailed information about the running Cloud Run …
  * cloudrun_get_system_status                        Provides a high-level dashboard of Cloud Run system status.
    cloudrun_get_endpoint                             Returns the active Cloud Run vLLM service URL if available.
    cloudrun_run_benchmark                            Runs a performance/concurrency benchmark sweep against the …
    cloudrun_analyze_gpu_logs                         Fetches Cloud Run logs for the specified service and uses G…
    cloudrun_get_help                                 Provides help text and summarizes the configuration options…
    cloudrun_get_metrics                              Fetches the Prometheus metrics from the active Cloud Run vL…
  (* = called by this demo, which only calls read-only tools)

== tools/call cloudrun_get_system_status =============================
  arguments              {}
  latency                2791 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    ### 🌀 GPU Cloud Run System Status (`gpu-2b-l4-devops-agent`)
    - **vLLM Health:** 🟢 Online (https://<your-service>.a.run.app)
    - **Cloud Run Service Status:** 🟢 Ready
    **👉 Next Step:** Use `cloudrun_query_gemma4` to interact with the model.

== tools/call cloudrun_get_model_details =============================
  arguments              {}
  latency                1705 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    ### 🧩 Model Details (https://<your-service>.a.run.app)
    
    **Model Information (`/v1/models`):**
    ```json
    [
      {
        "id": "/mnt/models/gemma-4-E2B-it",
        "object": "model",
        "owned_by": "vllm"
      }
    ]
    ```
    **Health Status (`/health`):**
    - Status: `Healthy` ✅

== tools/call cloudrun_query_gemma4_with_stats =======================
  arguments              {"prompt":"In one sentence, what is a TPU?"}
  latency                1333 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    ### 📊 Performance Stats
    - **Model:** `/mnt/models/gemma-4-E2B-it`
    - **Time to First Token (TTFT):** `0.086s`
    - **Total Generation Time:** `0.688s`
    - **Tokens per Second:** `53.11 tokens/s`
    - **Total Tokens (approx.):** `32`
    
    ### 💬 Model Response
    A TPU (Tensor Processing Unit) is a specialized type of integrated circuit designed to accelerate machine learning workloads, particularly those involving tensor operations common in deep learning.<turn|>

== Server log (stderr) ===============================================
  2026-09-11 11:41:32,212 - vllm-devops-agent - INFO - Initializing DevOps Agent MCP Server...
  2026-09-11 11:41:32,260 - vllm-devops-agent - INFO - Attempting to discover vLLM URL for service: gpu-2b-l4-devops-agent
  2026-09-11 11:41:33,284 - vllm-devops-agent - INFO - 📡 Automatically discovered vLLM at: https://<your-service>.a.run.app
  2026-09-11 11:41:34,115 - httpx - INFO - HTTP Request: GET https://<your-service>.a.run.app/health "HTTP/1.1 200 OK"
  2026-09-11 11:41:36,087 - httpx2 - INFO - HTTP Request: GET https://<your-service>.a.run.app/v1/models "HTTP/1.1 200 OK"
  2026-09-11 11:41:36,754 - httpx - INFO - HTTP Request: GET https://<your-service>.a.run.app/health "HTTP/1.1 200 OK"
  2026-09-11 11:41:36,756 - vllm-devops-agent - INFO - Querying model with stats with prompt: 'In one sentence, what is a TPU?...'
  2026-09-11 11:41:37,399 - httpx2 - INFO - HTTP Request: GET https://<your-service>.a.run.app/v1/models "HTTP/1.1 200 OK"
  2026-09-11 11:41:37,479 - httpx2 - INFO - HTTP Request: POST https://<your-service>.a.run.app/v1/chat/completions "HTTP/1.1 200 OK"
  2026-09-11 11:41:38,087 - vllm-devops-agent - INFO - Model response with stats: TTFT=0.086s, TotalTime=0.688s
```

When the local model runs out of `--max-tokens` mid-thought (`--no-status --max-tokens 32`):

```text
== tools/call query_model ============================================
  arguments              {"max_tokens":32,"prompt":"In one sentence, what is a TPU?"}
  latency                503 ms
  isError                false
  structuredContent      {"result": …} (same text as the content below)
  result:
    📡 **Reasoning only — no answer yet.** `finish_reason: length` after 32 tokens, all of them thinking.
    
    This is Gemma 4 reasoning, not a broken server. Re-run with a larger `max_tokens` (currently 32).
    ...
  No answer: the model was still reasoning when it stopped. Raise --max-tokens.
```
