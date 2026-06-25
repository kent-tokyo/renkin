//! renkin-mcp — MCP server for retrosynthesis via the Model Context Protocol.
//!
//! Transport: JSON-RPC 2.0 over stdio (one JSON object per line).
//! Register in Claude Desktop's `claude_desktop_config.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "renkin": { "command": "/path/to/renkin-mcp" }
//!   }
//! }
//! ```
#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env::{self, elem_symbols_to_mask};
use renkin::display::format_route_tree;
use renkin::search::{self, SearchConfig};
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = msg["id"].clone();
        let method = msg["method"].as_str().unwrap_or("");

        // Notifications have no id and require no response.
        if method.starts_with("notifications/") {
            continue;
        }

        let result = match method {
            "initialize" => handle_initialize(),
            "tools/list" => handle_tools_list(),
            "tools/call" => handle_tools_call(&msg),
            _ => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "Method not found"}
                });
                let _ = writeln!(out, "{resp}");
                let _ = out.flush();
                continue;
            }
        };

        let resp = json!({"jsonrpc": "2.0", "id": id, "result": result});
        let _ = writeln!(out, "{resp}");
        let _ = out.flush();
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "renkin", "version": VERSION}
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [{
            "name": "find_routes",
            "description": "Find retrosynthetic routes for a target molecule back to commercially available building blocks. Uses A* / AND-OR tree search with 5,000 SMIRKS templates (if data/templates_extracted_5000.smi is present next to the binary) and 509 curated building blocks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "smiles": {
                        "type": "string",
                        "description": "Target molecule as a SMILES string (e.g. \"CC(=O)Oc1ccccc1C(=O)O\" for aspirin)"
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Maximum retrosynthesis depth (default: 5)"
                    },
                    "max_routes": {
                        "type": "integer",
                        "description": "Maximum number of routes to return (default: 5)"
                    },
                    "avoid_elements": {
                        "type": "string",
                        "description": "Comma-separated element symbols to exclude from building blocks (e.g. \"Br,I\")"
                    },
                    "require_elements": {
                        "type": "string",
                        "description": "Comma-separated element symbols that must appear in at least one building block (e.g. \"B\" for Suzuki-type routes)"
                    }
                },
                "required": ["smiles"]
            }
        }]
    })
}

fn handle_tools_call(msg: &Value) -> Value {
    let args = &msg["params"]["arguments"];
    let smiles = match args["smiles"].as_str() {
        Some(s) => s,
        None => return tool_error("missing required argument: smiles"),
    };

    let depth = args["depth"].as_u64().unwrap_or(5) as u32;
    let max_routes = args["max_routes"].as_u64().unwrap_or(5) as usize;
    let avoid = args["avoid_elements"].as_str().unwrap_or("");
    let require = args["require_elements"].as_str().unwrap_or("");

    let env = chem_env::ChemEnv::load("data/building_blocks.smi")
        .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS));

    let mut rules = chem_env::default_rules();
    if std::path::Path::new("data/templates_extracted_5000.smi").is_file() {
        rules.extend(chem_env::load_rules_from_file(
            "data/templates_extracted_5000.smi",
        ));
    }

    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        forbidden_elements: elem_symbols_to_mask(avoid),
        required_element_present: elem_symbols_to_mask(require),
        ..Default::default()
    };

    let routes = match search::find_routes(smiles, &env, &rules, &config) {
        Ok(r) => r,
        Err(e) => return tool_error(&format!("search error: {e}")),
    };

    let mut text = format!("Target: {smiles}\nRoutes found: {}\n\n", routes.len());
    if routes.is_empty() {
        text.push_str(
            "No routes found. Try increasing depth, or remove element constraints if set.",
        );
    } else {
        for (i, route) in routes.iter().enumerate() {
            text.push_str(&format_route_tree(route, smiles, i + 1));
            text.push_str(&format!(
                "  Building blocks: {}\n\n",
                route.building_blocks.join(", ")
            ));
        }
    }

    json!({"content": [{"type": "text", "text": text}]})
}

fn tool_error(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}
