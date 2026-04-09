use std::path::PathBuf;

use anyhow::{anyhow, bail};
use clap::{Parser, Subcommand};
use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

/// Manage mcp-tool-server configuration files.
#[derive(Parser)]
#[command(name = "mcp-config", version)]
struct Cli {
    /// Path to the config.toml file.
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List all configured tools.
    List,

    /// Add an exec tool (runs a subprocess).
    AddExec {
        /// Unique tool name.
        name: String,
        /// Human-readable description shown to Claude.
        #[arg(short, long)]
        description: String,
        /// Executable to run.
        #[arg(short, long)]
        executable: String,
        /// Base arguments (may contain {param} placeholders). Repeat for multiple.
        #[arg(short, long = "arg")]
        args: Vec<String>,
        /// Environment variables as KEY=VALUE. Repeat for multiple.
        #[arg(long = "env")]
        env: Vec<String>,
        /// Working directory for the subprocess.
        #[arg(long)]
        working_dir: Option<String>,
        /// Timeout in seconds (default: 30).
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },

    /// Add a value tool (returns a static string).
    AddValue {
        /// Unique tool name.
        name: String,
        /// Human-readable description shown to Claude.
        #[arg(short, long)]
        description: String,
        /// The static value to return.
        #[arg(short, long)]
        value: String,
    },

    /// Add a parameter to an existing exec tool.
    AddParam {
        /// Tool name to add the param to.
        tool: String,
        /// Parameter name (used in {placeholders}).
        #[arg(short, long)]
        name: String,
        /// Description shown to Claude.
        #[arg(short, long)]
        description: Option<String>,
        /// Whether this param is required.
        #[arg(short, long)]
        required: bool,
        /// Default value if not provided.
        #[arg(long)]
        default: Option<String>,
    },

    /// Remove a tool by name.
    Remove {
        /// Tool name to remove.
        name: String,
    },

    /// Show full details of a single tool.
    Show {
        /// Tool name to inspect.
        name: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let path = &cli.config;

    match cli.command {
        Command::List => cmd_list(path),
        Command::AddExec { name, description, executable, args, env, working_dir, timeout_secs } => {
            cmd_add_exec(path, &name, &description, &executable, &args, &env, working_dir.as_deref(), timeout_secs)
        }
        Command::AddValue { name, description, value } => {
            cmd_add_value(path, &name, &description, &value)
        }
        Command::AddParam { tool, name, description, required, default } => {
            cmd_add_param(path, &tool, &name, description.as_deref(), required, default.as_deref())
        }
        Command::Remove { name } => cmd_remove(path, &name),
        Command::Show { name } => cmd_show(path, &name),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_doc(path: &PathBuf) -> anyhow::Result<DocumentMut> {
    if path.exists() {
        let src = std::fs::read_to_string(path)?;
        Ok(src.parse::<DocumentMut>()?)
    } else {
        Ok(DocumentMut::new())
    }
}

fn write_doc(path: &PathBuf, doc: &DocumentMut) -> anyhow::Result<()> {
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

fn tools_table(doc: &mut DocumentMut) -> &mut Table {
    if !doc.contains_key("tools") {
        doc["tools"] = Item::Table(Table::new());
    }
    doc["tools"].as_table_mut().expect("tools is a table")
}

// ── commands ──────────────────────────────────────────────────────────────────

fn cmd_list(path: &PathBuf) -> anyhow::Result<()> {
    let doc = read_doc(path)?;
    let tools = match doc.get("tools").and_then(|t| t.as_table()) {
        Some(t) => t,
        None => {
            println!("No tools configured.");
            return Ok(());
        }
    };

    if tools.is_empty() {
        println!("No tools configured.");
        return Ok(());
    }

    for (name, item) in tools {
        let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = item.get("description").and_then(|v| v.as_str()).unwrap_or("");
        println!("{name:20} [{kind}]  {desc}");
    }
    Ok(())
}

fn cmd_add_exec(
    path: &PathBuf,
    name: &str,
    description: &str,
    executable: &str,
    args: &[String],
    env_pairs: &[String],
    working_dir: Option<&str>,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let mut doc = read_doc(path)?;

    if tools_table(&mut doc).contains_key(name) {
        bail!("Tool '{name}' already exists. Remove it first.");
    }

    let mut t = Table::new();
    t["kind"] = value("exec");
    t["description"] = value(description);
    t["executable"] = value(executable);

    let mut arr = Array::new();
    for a in args {
        arr.push(a.as_str());
    }
    t["base_args"] = Item::Value(Value::Array(arr));

    if !env_pairs.is_empty() {
        let mut env_table = Table::new();
        for pair in env_pairs {
            let (k, v) = pair.split_once('=').ok_or_else(|| anyhow!("env must be KEY=VALUE, got: {pair}"))?;
            env_table[k] = value(v);
        }
        t["env"] = Item::Table(env_table);
    }

    if let Some(dir) = working_dir {
        t["working_dir"] = value(dir);
    }

    if timeout_secs != 30 {
        t["timeout_secs"] = value(timeout_secs as i64);
    }

    tools_table(&mut doc)[name] = Item::Table(t);
    write_doc(path, &doc)?;
    println!("Added exec tool '{name}'.");
    Ok(())
}

fn cmd_add_value(path: &PathBuf, name: &str, description: &str, val: &str) -> anyhow::Result<()> {
    let mut doc = read_doc(path)?;

    if tools_table(&mut doc).contains_key(name) {
        bail!("Tool '{name}' already exists. Remove it first.");
    }

    let mut t = Table::new();
    t["kind"] = value("value");
    t["description"] = value(description);
    t["value"] = value(val);

    tools_table(&mut doc)[name] = Item::Table(t);
    write_doc(path, &doc)?;
    println!("Added value tool '{name}'.");
    Ok(())
}

fn cmd_add_param(
    path: &PathBuf,
    tool_name: &str,
    param_name: &str,
    description: Option<&str>,
    required: bool,
    default: Option<&str>,
) -> anyhow::Result<()> {
    let mut doc = read_doc(path)?;

    let tool = doc
        .get_mut("tools")
        .and_then(|t| t.as_table_mut())
        .and_then(|t| t.get_mut(tool_name))
        .and_then(|t| t.as_table_mut())
        .ok_or_else(|| anyhow!("Tool '{tool_name}' not found"))?;

    let kind = tool.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    if kind != "exec" {
        bail!("Tool '{tool_name}' is not an exec tool; only exec tools have params");
    }

    if !tool.contains_key("params") {
        tool["params"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }

    let params = tool["params"]
        .as_array_of_tables_mut()
        .ok_or_else(|| anyhow!("'params' is not an array of tables"))?;

    let mut p = Table::new();
    p["name"] = value(param_name);
    if let Some(desc) = description {
        p["description"] = value(desc);
    }
    p["required"] = value(required);
    if let Some(def) = default {
        p["default"] = value(def);
    }

    params.push(p);
    write_doc(path, &doc)?;
    println!("Added param '{param_name}' to tool '{tool_name}'.");
    Ok(())
}

fn cmd_remove(path: &PathBuf, name: &str) -> anyhow::Result<()> {
    let mut doc = read_doc(path)?;
    let tools = tools_table(&mut doc);
    if tools.remove(name).is_none() {
        bail!("Tool '{name}' not found");
    }
    write_doc(path, &doc)?;
    println!("Removed tool '{name}'.");
    Ok(())
}

fn cmd_show(path: &PathBuf, name: &str) -> anyhow::Result<()> {
    let doc = read_doc(path)?;
    let tool = doc
        .get("tools")
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(name))
        .ok_or_else(|| anyhow!("Tool '{name}' not found"))?;

    println!("[tools.{name}]");
    print!("{tool}");
    Ok(())
}
