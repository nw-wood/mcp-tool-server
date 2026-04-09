use std::collections::HashMap;
use std::time::Duration;

use anyhow::anyhow;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;

use crate::config::{Config, ExecTool, ParamDef, ToolDef};

pub struct ToolRegistry {
    tools: HashMap<String, ToolDef>,
}

impl ToolRegistry {
    pub fn from_config(config: Config) -> Self {
        Self { tools: config.tools }
    }

    pub fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|(name, def)| {
                let schema = match def {
                    ToolDef::Exec(t) => build_schema(&t.params),
                    ToolDef::Value(_) => json!({ "type": "object", "properties": {} }),
                };
                json!({
                    "name": name,
                    "description": def.description(),
                    "inputSchema": schema,
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    pub async fn call(&self, name: &str, args: &Value) -> anyhow::Result<String> {
        match self.tools.get(name) {
            Some(ToolDef::Exec(t)) => run_exec(t, args).await,
            Some(ToolDef::Value(t)) => Ok(t.value.clone()),
            None => Err(anyhow!("Unknown tool: {}", name)),
        }
    }
}

async fn run_exec(tool: &ExecTool, args: &Value) -> anyhow::Result<String> {
    let resolved = resolve_params(&tool.params, args)?;
    let rendered_args: Vec<String> =
        tool.base_args.iter().map(|a| substitute(a, &resolved)).collect();

    eprintln!("[mcp-tool-server] exec: {} {:?}", tool.executable, rendered_args);

    let mut cmd = Command::new(&tool.executable);
    cmd.args(&rendered_args);
    cmd.envs(&tool.env);
    if let Some(dir) = &tool.working_dir {
        cmd.current_dir(dir);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let fut = cmd.output();
    let output = timeout(Duration::from_secs(tool.timeout_secs), fut)
        .await
        .map_err(|_| anyhow!("'{}' timed out after {}s", tool.executable, tool.timeout_secs))??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !output.status.success() {
        result.push_str(&format!("[exit {}]\n", output.status));
    }
    result.push_str(&stdout);
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push_str("\n--- stderr ---\n");
        }
        result.push_str(&stderr);
    }

    Ok(result)
}

fn resolve_params(
    defs: &[ParamDef],
    args: &Value,
) -> anyhow::Result<HashMap<String, String>> {
    let mut resolved = HashMap::new();
    for p in defs {
        let provided = args.get(&p.name).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        });
        match provided.or_else(|| p.default.clone()) {
            Some(val) => {
                resolved.insert(p.name.clone(), val);
            }
            None if p.required => {
                return Err(anyhow!("Required parameter '{}' was not provided", p.name));
            }
            None => {}
        }
    }
    Ok(resolved)
}

fn substitute(template: &str, values: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (k, v) in values {
        result = result.replace(&format!("{{{}}}", k), v);
    }
    result
}

fn build_schema(params: &[ParamDef]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for p in params {
        properties.insert(
            p.name.clone(),
            json!({
                "type": "string",
                "description": p.description.as_deref().unwrap_or(""),
            }),
        );
        if p.required {
            required.push(p.name.clone());
        }
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}
