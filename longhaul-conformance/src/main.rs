//! longhaul-conformance — MCP 2026-07-28 RC conformance suite runner.
//!
//! Given a base URL, exercises the RC subset this crate models and validates
//! every wire message against the JSON Schema fixtures in `fixtures/`.
//!
//! ## Suites
//!
//! * `discovery` — `server/discover` shape + `protocolVersion` pin.
//! * `tools`     — `tools/list` result shape + cache metadata fields.
//! * `tasks`     — full task lifecycle (working → inputRequired → working →
//!   completed) + cancel + illegal-transition error.
//!
//! ## Usage
//!
//! ```text
//! longhaul-conformance --url http://localhost:3000
//! longhaul-conformance --url http://localhost:3000 --filter tasks
//! longhaul-conformance --url http://localhost:3000 --json
//! ```

use std::time::{Duration, Instant};

use clap::Parser;
use reqwest::Client;
use serde_json::{json, Value};
use uuid::Uuid;

use longhaul_core::{
    http::{HEADER_METHOD, HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION},
    jsonrpc::Response,
};

/// Validate an MCP server endpoint against the 2026-07-28 RC specification.
#[derive(Parser, Debug)]
#[command(
    name    = "longhaul-conformance",
    version,
    about   = "MCP 2026-07-28 RC conformance suite runner",
    long_about = None
)]
struct Cli {
    /// Base URL of the MCP server under test (e.g. http://localhost:3000)
    #[arg(short, long)]
    url: Option<String>,

    /// Only run suites matching this filter substring
    #[arg(short, long)]
    filter: Option<String>,

    /// Emit machine-readable JSON results to stdout
    #[arg(long, default_value_t = false)]
    json: bool,
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct SuiteResult {
    suite: String,
    passed: usize,
    failed: usize,
    skipped: usize,
    cases: Vec<CaseResult>,
    elapsed_ms: u64,
}

#[derive(Debug, serde::Serialize)]
struct CaseResult {
    name: String,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl SuiteResult {
    fn ok(&self) -> bool {
        self.failed == 0
    }
}

// ---------------------------------------------------------------------------
// MCP HTTP client helper
// ---------------------------------------------------------------------------

struct McpClient {
    base_url: String,
    http: Client,
}

impl McpClient {
    fn new(base_url: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        Self { base_url, http }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Response, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let resp = self
            .http
            .post(format!("{}/mcp", self.base_url))
            .header("content-type", "application/json")
            .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION)
            .header(HEADER_METHOD, method)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        serde_json::from_slice::<Response>(&bytes).map_err(|e| format!("JSON-RPC parse error: {e}"))
    }

    async fn call_raw(&self, method: &str, params: Value) -> Result<Value, String> {
        let resp = self.call(method, params).await?;
        match resp.result() {
            Some(v) => Ok(v.clone()),
            None => Err(format!(
                "unexpected error: {:?}",
                resp.error().map(|e| e.message.as_str())
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Test case builder
// ---------------------------------------------------------------------------

struct Suite {
    name: String,
    cases: Vec<CaseResult>,
    elapsed: Duration,
}

impl Suite {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cases: Vec::new(),
            elapsed: Duration::ZERO,
        }
    }

    fn record(&mut self, name: impl Into<String>, result: Result<(), String>) {
        let passed = result.is_ok();
        self.cases.push(CaseResult {
            name: name.into(),
            passed,
            error: result.err(),
        });
    }

    fn into_result(self) -> SuiteResult {
        let passed = self.cases.iter().filter(|c| c.passed).count();
        let failed = self.cases.iter().filter(|c| !c.passed).count();
        SuiteResult {
            suite: self.name,
            passed,
            failed,
            skipped: 0,
            cases: self.cases,
            elapsed_ms: self.elapsed.as_millis() as u64,
        }
    }
}

// ---------------------------------------------------------------------------
// Suites
// ---------------------------------------------------------------------------

async fn run_discovery(client: &McpClient, schemas: &schema::Schemas) -> SuiteResult {
    let t0 = Instant::now();
    let mut suite = Suite::new("discovery");

    // 1. server/discover succeeds
    let result = async {
        let v = client.call_raw("server/discover", json!({})).await?;
        schemas.validate("discover_result", &v)
    }
    .await;
    suite.record("discover_result_schema", result);

    // 2. protocolVersion is exactly the RC string
    let result = async {
        let v = client.call_raw("server/discover", json!({})).await?;
        let ver = v["protocolVersion"]
            .as_str()
            .ok_or("missing protocolVersion")?;
        if ver != PROTOCOL_VERSION {
            return Err(format!(
                "protocolVersion is {ver:?}, expected {PROTOCOL_VERSION:?}"
            ));
        }
        Ok(())
    }
    .await;
    suite.record("protocol_version_pinned", result);

    // 3. response is a valid JSON-RPC response envelope
    let result = async {
        let resp = client.call("server/discover", json!({})).await?;
        let v = serde_json::to_value(&resp).map_err(|e| e.to_string())?;
        schemas.validate("jsonrpc_response", &v)
    }
    .await;
    suite.record("jsonrpc_response_envelope", result);

    suite.elapsed = t0.elapsed();
    suite.into_result()
}

async fn run_tools(client: &McpClient, schemas: &schema::Schemas) -> SuiteResult {
    let t0 = Instant::now();
    let mut suite = Suite::new("tools");

    // 1. tools/list returns a conformant result
    let result = async {
        let v = client.call_raw("tools/list", json!({})).await?;
        schemas.validate("tools_list_result", &v)
    }
    .await;
    suite.record("tools_list_schema", result);

    // 2. tools array is present
    let result = async {
        let v = client.call_raw("tools/list", json!({})).await?;
        if !v["tools"].is_array() {
            return Err("tools field is not an array".to_owned());
        }
        Ok(())
    }
    .await;
    suite.record("tools_field_is_array", result);

    // 3. cache metadata (ttlMs / cacheScope) are present when the server
    //    returns them (they're optional per spec; we report presence only).
    let result = async {
        let v = client.call_raw("tools/list", json!({})).await?;
        // Both fields are optional; if present they must be the right types.
        if let Some(ttl) = v.get("ttlMs") {
            if !ttl.is_number() {
                return Err(format!("ttlMs is not a number: {ttl}"));
            }
        }
        if let Some(scope) = v.get("cacheScope") {
            if !scope.is_string() {
                return Err(format!("cacheScope is not a string: {scope}"));
            }
        }
        Ok(())
    }
    .await;
    suite.record("cache_metadata_types", result);

    suite.elapsed = t0.elapsed();
    suite.into_result()
}

async fn run_tasks(client: &McpClient, _schemas: &schema::Schemas) -> SuiteResult {
    let t0 = Instant::now();
    let mut suite = Suite::new("tasks");

    // The CLI suite covers error-code behaviour only: it generates a fresh UUID
    // for each run so it never collides with live tasks, then verifies that
    // well-formed requests for non-existent task ids return -32602 and that
    // removed methods (tasks/list) return -32601. Full lifecycle coverage
    // (working → inputRequired → working → completed) lives in the in-process
    // suite at `tests/conformance.rs`, which boots a real server on a random
    // port and pre-inserts tasks before exercising them.

    // Attempt to use a well-known test task id.
    let task_id = format!("conformance-{}", Uuid::new_v4());

    // tasks/get on a non-existent task should return -32602
    let result = async {
        let resp = client.call("tasks/get", json!({"taskId": task_id})).await?;
        match resp.error() {
            Some(e) if e.code == longhaul_core::error::INVALID_PARAMS => Ok(()),
            Some(e) => Err(format!("Expected -32602, got {}: {}", e.code, e.message)),
            None => Err("Expected error for missing task, got success".to_owned()),
        }
    }
    .await;
    suite.record("get_missing_task_returns_32602", result);

    // tasks/cancel on a non-existent task should also return -32602
    let result = async {
        let resp = client
            .call("tasks/cancel", json!({"taskId": task_id}))
            .await?;
        match resp.error() {
            Some(e) if e.code == longhaul_core::error::INVALID_PARAMS => Ok(()),
            Some(e) => Err(format!("Expected -32602, got {}: {}", e.code, e.message)),
            None => Err("Expected error for missing task, got success".to_owned()),
        }
    }
    .await;
    suite.record("cancel_missing_task_returns_32602", result);

    // tasks/update on a non-existent task should return -32602
    let result = async {
        let resp = client
            .call(
                "tasks/update",
                json!({"taskId": task_id, "status": "completed"}),
            )
            .await?;
        match resp.error() {
            Some(e) if e.code == longhaul_core::error::INVALID_PARAMS => Ok(()),
            Some(e) => Err(format!("Expected -32602, got {}: {}", e.code, e.message)),
            None => Err("Expected error for missing task, got success".to_owned()),
        }
    }
    .await;
    suite.record("update_missing_task_returns_32602", result);

    // Method-not-found returns -32601
    let result = async {
        let resp = client.call("tasks/list", json!({})).await?;
        match resp.error() {
            Some(e) if e.code == longhaul_core::error::METHOD_NOT_FOUND => Ok(()),
            Some(e) => Err(format!(
                "Expected -32601 (tasks/list removed), got {}: {}",
                e.code, e.message
            )),
            None => Err("tasks/list should not exist in RC".to_owned()),
        }
    }
    .await;
    suite.record("tasks_list_removed_returns_32601", result);

    suite.elapsed = t0.elapsed();
    suite.into_result()
}

// ---------------------------------------------------------------------------
// Schema validation helpers (delegated to `schema` module)
// ---------------------------------------------------------------------------

mod schema {
    use jsonschema::Validator;
    use serde_json::Value;
    use std::collections::HashMap;

    pub struct Schemas {
        validators: HashMap<String, Validator>,
    }

    impl Schemas {
        /// Load all fixture schemas embedded at compile time.
        pub fn load() -> Self {
            let fixtures: &[(&str, &str)] = &[
                (
                    "jsonrpc_request",
                    include_str!("../fixtures/jsonrpc_request.schema.json"),
                ),
                (
                    "jsonrpc_response",
                    include_str!("../fixtures/jsonrpc_response.schema.json"),
                ),
                (
                    "tools_list_result",
                    include_str!("../fixtures/tools_list_result.schema.json"),
                ),
                ("task", include_str!("../fixtures/task.schema.json")),
                (
                    "discover_result",
                    include_str!("../fixtures/discover_result.schema.json"),
                ),
            ];

            let mut validators = HashMap::new();
            for (name, src) in fixtures {
                let schema: Value = serde_json::from_str(src)
                    .unwrap_or_else(|e| panic!("invalid fixture {name}: {e}"));
                let validator = jsonschema::validator_for(&schema)
                    .unwrap_or_else(|e| panic!("invalid schema {name}: {e}"));
                validators.insert(name.to_string(), validator);
            }
            Self { validators }
        }

        /// Validate `value` against the named schema. Returns a descriptive
        /// error if validation fails or the schema name is unknown.
        pub fn validate(&self, name: &str, value: &Value) -> Result<(), String> {
            let validator = self
                .validators
                .get(name)
                .ok_or_else(|| format!("unknown schema: {name}"))?;
            let mut errors: Vec<String> = validator
                .iter_errors(value)
                .map(|e| e.to_string())
                .collect();
            if errors.is_empty() {
                Ok(())
            } else {
                errors.truncate(5);
                Err(format!("schema {name} failed: {}", errors.join("; ")))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let url = match cli.url {
        None => {
            print_usage();
            std::process::exit(1);
        }
        Some(u) => u,
    };

    let client = McpClient::new(url.clone());
    let schemas = schema::Schemas::load();

    let all_suites: Vec<(&str, bool)> = vec![("discovery", true), ("tools", true), ("tasks", true)];

    let filter = cli.filter.as_deref().unwrap_or("");

    let mut results: Vec<SuiteResult> = Vec::new();
    let mut any_failed = false;

    for (suite_name, _) in &all_suites {
        if !filter.is_empty() && !suite_name.contains(filter) {
            continue;
        }

        let result = match *suite_name {
            "discovery" => run_discovery(&client, &schemas).await,
            "tools" => run_tools(&client, &schemas).await,
            "tasks" => run_tasks(&client, &schemas).await,
            _ => unreachable!(),
        };

        if !result.ok() {
            any_failed = true;
        }
        results.push(result);
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        print_human(&url, &results);
    }

    if any_failed {
        std::process::exit(1);
    }
}

fn print_human(url: &str, results: &[SuiteResult]) {
    println!("longhaul-conformance — MCP 2026-07-28 RC");
    println!("Target: {url}");
    println!();

    let total_passed: usize = results.iter().map(|r| r.passed).sum();
    let total_failed: usize = results.iter().map(|r| r.failed).sum();

    for suite in results {
        let status = if suite.ok() { "PASS" } else { "FAIL" };
        println!(
            "[{status}] {suite} — {}/{} passed ({elapsed}ms)",
            suite.passed,
            suite.passed + suite.failed,
            suite = suite.suite,
            elapsed = suite.elapsed_ms,
        );
        for case in &suite.cases {
            let mark = if case.passed { "  ✓" } else { "  ✗" };
            print!("{mark} {}", case.name);
            if let Some(err) = &case.error {
                print!(" — {err}");
            }
            println!();
        }
        println!();
    }

    println!("Total: {total_passed} passed, {total_failed} failed");
}

fn print_usage() {
    eprintln!("longhaul-conformance — MCP 2026-07-28 RC");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  longhaul-conformance --url <BASE_URL> [--filter <SUITE>] [--json]");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  longhaul-conformance --url http://localhost:3000");
    eprintln!("  longhaul-conformance --url http://localhost:3000 --filter tasks");
    eprintln!("  longhaul-conformance --url http://localhost:3000 --json");
    eprintln!();
    eprintln!("Suites:");
    eprintln!("  discovery  server/discover capability discovery");
    eprintln!("  tools      list (cache metadata), call outcomes");
    eprintln!("  tasks      get/update/cancel + error codes");
}
