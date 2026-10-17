use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub app: AppSection,
    pub build: Option<BuildSection>,
    pub routing: RoutingSection,
    pub service: ServiceSection,
    pub resources: Option<ResourcesSection>,
    pub migrations: Option<MigrationsSection>,
    pub compute: Option<ComputeSection>,
    pub start: Option<StartSection>,
}

#[derive(Debug, Deserialize)]
pub struct AppSection {
    pub id: String,
    pub display_name: Option<String>,
    pub runtime: String,
    pub dockerfile: String,
}

#[derive(Debug, Deserialize)]
pub struct BuildSection {
    #[serde(default = "default_dot")]
    pub context_dir: String,
    pub picusbase_context: Option<String>,
    #[serde(default)]
    pub args: std::collections::HashMap<String, String>,
}

fn default_dot() -> String {
    ".".to_string()
}

#[derive(Debug, Deserialize)]
pub struct RoutingSection {
    #[serde(default = "default_prefix_mode")]
    pub mode: String,
    pub prefix: Option<String>,
    pub host: Option<String>,
    #[serde(default = "default_true")]
    pub strip_prefix: bool,
}

fn default_prefix_mode() -> String {
    "prefix".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ServiceSection {
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct ResourcesSection {
    pub replicas: Option<u32>,
    pub cpu_request: Option<String>,
    pub cpu_limit: Option<String>,
    pub mem_request: Option<String>,
    pub mem_limit: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MigrationsSection {
    pub dir: String,
    #[serde(default)]
    pub files: Vec<String>,
    pub schema_name: Option<String>,
    #[serde(default)]
    pub setup_background_jobs: bool,
}

#[derive(Debug, Deserialize)]
pub struct ComputeSection {
    pub functions_dir: Option<String>,
    #[serde(default)]
    pub business_domains: Vec<String>,
    #[serde(default)]
    pub provisioning_functions: Vec<String>,
    pub post_deploy_sql: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StartSection {
    #[serde(default)]
    pub values_patch: std::collections::HashMap<String, String>,
}

impl AppConfig {
    pub fn load(config_path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(config_path).with_context(|| {
            format!("Failed to read app.toml at {}", config_path.display())
        })?;
        let config: AppConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse app.toml at {}", config_path.display()))?;
        Ok(config)
    }

    /// Returns the app-id converted to camelCase for use as Helm values key.
    /// e.g. "erp-launcher" -> "erpLauncher"
    pub fn helm_key(&self) -> String {
        camel_case(&self.app.id)
    }

    /// Resolve absolute path to the app.toml directory.
    pub fn app_dir(config_path: &Path) -> PathBuf {
        config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Resolve the picusbase build context path relative to app dir.
    pub fn resolve_picusbase_context(&self, app_dir: &Path) -> Option<PathBuf> {
        if let Some(ref ctx) = self.build.as_ref().and_then(|b| b.picusbase_context.clone()) {
            let p = app_dir.join(ctx);
            return Some(p);
        }
        // Auto-detect: walk up until we find a sibling picusbase/ dir
        let mut current = app_dir.to_path_buf();
        loop {
            let candidate = current.join("picusbase");
            if candidate.is_dir() {
                return Some(candidate);
            }
            if let Some(parent) = current.parent() {
                if parent == current {
                    break;
                }
                current = parent.to_path_buf();
            } else {
                break;
            }
        }
        None
    }
}

pub fn camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for (i, c) in s.chars().enumerate() {
        if c == '-' || c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_uppercase().next().unwrap_or(c));
            capitalize_next = false;
        } else if i == 0 {
            result.push(c.to_lowercase().next().unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camel_case() {
        assert_eq!(camel_case("erp-launcher"), "erpLauncher");
        assert_eq!(camel_case("my-app"), "myApp");
        assert_eq!(camel_case("app"), "app");
    }
}
