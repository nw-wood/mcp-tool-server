use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub tools: HashMap<String, ToolDef>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ToolDef {
    Exec(ExecTool),
    Value(ValueTool),
}

impl ToolDef {
    pub fn description(&self) -> &str {
        match self {
            Self::Exec(t) => &t.description,
            Self::Value(t) => &t.description,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ExecTool {
    pub description: String,
    pub executable: String,
    #[serde(default)]
    pub base_args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub working_dir: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub params: Vec<ParamDef>,
}

#[derive(Debug, Deserialize)]
pub struct ValueTool {
    pub description: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ParamDef {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    pub default: Option<String>,
}

fn default_timeout() -> u64 {
    30
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config '{}': {}", path, e))?;
        toml::from_str(&content).map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))
    }
}
