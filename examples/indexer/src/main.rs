// examples/indexer — placeholder
//
// This binary will index a local directory and serve its contents as MCP resources
// via longhaul-server.  Phase A: scaffold only.
//
// TODO(Phase B):
//   1. Parse CLI args: `--dir <PATH>` + `--bind <ADDR>`.
//   2. Walk the directory, building a resource list.
//   3. Start a longhaul-server::router with the resource list wired in.
//   4. Serve until Ctrl-C.

#[tokio::main]
async fn main() {
    println!("longhaul indexer example — scaffold placeholder");
    println!("Implementation arrives in Phase B.");
}
