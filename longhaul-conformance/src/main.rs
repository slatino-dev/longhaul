// longhaul-conformance — MCP 2026-07-28 RC conformance runner
//
// Phase A: scaffold only.  The runner prints usage and exits.
// Phase B will add test suites under the `suites/` module.

use clap::Parser;

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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.url {
        None => {
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
            eprintln!("Suites (stubs — runner implementation coming in a later phase):");
            eprintln!("  discovery      server/discover capability discovery");
            eprintln!("  resources      list, read, subscribe");
            eprintln!("  tools          list (cache metadata), call, outcomes");
            eprintln!("  prompts        list, get");
            eprintln!("  sampling       createMessage round-trip");
            eprintln!("  tasks          get/update/cancel + task-handle calls (Tasks extension)");
            eprintln!();
            std::process::exit(1);
        }
        Some(url) => {
            // TODO(Phase B): dispatch to test suites, collect results, report pass/fail.
            println!("Target: {url}");
            if let Some(f) = cli.filter {
                println!("Filter: {f}");
            }
            println!("(Phase A scaffold — no tests implemented yet)");
        }
    }
}
