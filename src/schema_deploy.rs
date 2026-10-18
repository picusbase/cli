//! `picusbase deploy schema` — deploy the data layer of a bundle directory into a project:
//! `doctypes/<code>/*.yaml` (entities), `roles/<code>/role.yaml`, `functions/<code>/`,
//! `automations/*.json`, `ai_agent/agents/*.yaml`. UI lives in separate frontends
//! (`picusbase deploy app`).
//!
//! Role → permission resolution runs server-side (`POST .../schema/roles`, same crate the
//! platform validates with); the merged `permissions.yaml` files come back and are written to disk
//! before each entity is defined, drafted and published.
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::client::PicusClient;

pub struct SchemaDeployOptions {
    pub skip_functions: bool,
    pub no_publish: bool,
    pub bound_user_id: Option<String>,
}

struct Api<'a> {
    http: Client,
    base: String,
    project_ref: &'a str,
    token: String,
}

impl Api<'_> {
    fn url(&self, path: &str) -> String {
        format!("{}/platform/projects/{}{path}", self.base, self.project_ref)
    }

    fn send(&self, request: reqwest::blocking::RequestBuilder, what: &str) -> Result<Value> {
        let res = request.bearer_auth(&self.token).send().with_context(|| format!("{what}: request failed"))?;
        let status = res.status();
        let body = res.text().unwrap_or_default();
        if !status.is_success() {
            bail!("{what} failed ({status}): {body}");
        }
        Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    fn post(&self, path: &str, body: &Value, what: &str) -> Result<Value> {
        self.send(self.http.post(self.url(path)).json(body), what)
    }
}

type Files = BTreeMap<String, String>;

/// `<parent>/<code>/*.yaml` → code → (filename → content).
fn scan_yaml_dirs(parent: &Path) -> Result<BTreeMap<String, Files>> {
    let mut out = BTreeMap::new();
    if !parent.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(parent).with_context(|| format!("Failed to read {}", parent.display()))? {
        let dir = entry?.path();
        if !dir.is_dir() {
            continue;
        }
        let code = dir.file_name().unwrap().to_string_lossy().to_string();
        let mut files = Files::new();
        for file in std::fs::read_dir(&dir)? {
            let path = file?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                files.insert(path.file_name().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&path)?);
            }
        }
        if !files.is_empty() {
            out.insert(code, files);
        }
    }
    Ok(out)
}

fn yaml_to_json(text: &str, what: &str) -> Result<Value> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).with_context(|| format!("Invalid YAML in {what}"))?;
    Ok(serde_json::to_value(value)?)
}

/// Entities referenced by a `relation` field must be defined first (otherwise 422 on a fresh project).
fn deploy_order(doctypes: &BTreeMap<String, Value>) -> Vec<String> {
    let deps: BTreeMap<&str, BTreeSet<&str>> = doctypes
        .iter()
        .map(|(code, doc)| {
            let targets = doc["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|f| f["data_type"] == "relation")
                .filter_map(|f| f["validation_rules"]["target_entity_code"].as_str())
                .filter(|target| *target != code && doctypes.contains_key(*target))
                .collect();
            (code.as_str(), targets)
        })
        .collect();
    let mut order: Vec<String> = Vec::new();
    let mut done: HashSet<&str> = HashSet::new();
    loop {
        let ready: Vec<&str> = deps
            .iter()
            .filter(|(code, targets)| !done.contains(*code) && targets.iter().all(|t| done.contains(t)))
            .map(|(code, _)| *code)
            .collect();
        if ready.is_empty() {
            break;
        }
        for code in ready {
            done.insert(code);
            order.push(code.to_string());
        }
    }
    let cyclic: Vec<String> = deps.keys().filter(|c| !done.contains(*c)).map(|c| c.to_string()).collect();
    if !cyclic.is_empty() {
        println!("  (relation cycle between: {} — kept in name order)", cyclic.join(", "));
        order.extend(cyclic);
    }
    order
}

fn jwt_subject(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?.get("sub")?.as_str().map(str::to_string)
}

pub fn run(
    client: &PicusClient,
    bundle_dir: &Path,
    project_ref: &str,
    personal_token: Option<String>,
    options: SchemaDeployOptions,
) -> Result<()> {
    if !bundle_dir.is_dir() {
        bail!("Bundle directory not found: {}", bundle_dir.display());
    }
    let api = Api { http: Client::new(), base: client.base_url().trim_end_matches('/').to_string(), project_ref, token: client.bearer_token()? };

    let doctypes_dir = bundle_dir.join("doctypes");
    let mut entities = scan_yaml_dirs(&doctypes_dir)?;
    let roles: BTreeMap<String, String> = scan_yaml_dirs(&bundle_dir.join("roles"))?
        .into_iter()
        .filter_map(|(code, mut files)| files.remove("role.yaml").map(|text| (code, text)))
        .collect();
    println!("Found {} entities, {} roles in {}", entities.len(), roles.len(), bundle_dir.display());

    if !entities.is_empty() {
        println!("\nRoles & permissions:");
        let result = api.post("/schema/roles", &json!({ "entities": entities, "roles": roles }), "resolve roles")?;
        for role in result["roles"].as_array().into_iter().flatten() {
            let action = role["action"].as_str().unwrap_or("");
            if action != "exists" && action != "system" {
                println!("  {action} role '{}'", role["role"].as_str().unwrap_or("?"));
            }
        }
        for warning in result["warnings"].as_array().into_iter().flatten() {
            println!("  ⚠️  {}", warning.as_str().unwrap_or_default());
        }
        let generated: HashMap<String, String> = serde_json::from_value(result["generated_permissions"].clone()).unwrap_or_default();
        for (code, text) in &generated {
            if entities.get(code).and_then(|f| f.get("permissions.yaml")) != Some(text) {
                std::fs::write(doctypes_dir.join(code).join("permissions.yaml"), text)?;
                println!("  ~ doctypes/{code}/permissions.yaml regenerated from roles/");
            }
            entities.entry(code.clone()).or_default().insert("permissions.yaml".to_string(), text.clone());
        }

        let doctypes: BTreeMap<String, Value> = entities
            .iter()
            .map(|(code, files)| {
                let text = files.get("doctype.yaml").with_context(|| format!("doctypes/{code}/doctype.yaml is missing"))?;
                Ok((code.clone(), yaml_to_json(text, &format!("doctypes/{code}/doctype.yaml"))?))
            })
            .collect::<Result<_>>()?;

        println!("\nEntities:");
        let mut invalid = Vec::new();
        for code in deploy_order(&doctypes) {
            api.post("/entities", &doctypes[&code], &format!("define entity '{code}'"))?;
            let draft = api.post(
                &format!("/builder/entity/{code}/versions"),
                &json!({ "files": entities[&code] }),
                &format!("draft entity '{code}'"),
            )?;
            let seq = draft["version_seq"].as_i64().unwrap_or(-1);
            if draft["validation_status"] != "valid" {
                println!("  ✗ {code} v{seq}: {}", draft["validation_errors"]);
                invalid.push(code);
                continue;
            }
            if options.no_publish {
                println!("  ✓ {code} v{seq} drafted");
                continue;
            }
            let id = draft["id"].as_str().unwrap_or_default();
            api.post(&format!("/builder/entity/{code}/versions/{id}/publish"), &Value::Null, &format!("publish entity '{code}'"))?;
            println!("  ✓ {code} v{seq}");
        }
        if !invalid.is_empty() {
            bail!("{} entity version(s) failed validation: {}", invalid.len(), invalid.join(", "));
        }
    }

    if options.skip_functions {
        println!("\nFunctions: skipped (--skip-functions)");
    } else {
        deploy_functions(&bundle_dir.join("functions"), client, project_ref, personal_token)?;
    }
    restore_automations(&api, &bundle_dir.join("automations"))?;
    let bound_user = options.bound_user_id.or_else(|| jwt_subject(&api.token));
    restore_ai_agents(&api, &bundle_dir.join("ai_agent").join("agents"), bound_user.as_deref())?;

    println!("\nDone.");
    Ok(())
}

/// `picusbase data restore` — replace entity records from `<dir>/*.sql` dumps
/// (`DELETE FROM entity_*` + `INSERT INTO entity_*`). Entities must be deployed first.
pub fn restore_data(client: &PicusClient, dir: &Path, project_ref: &str) -> Result<()> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "sql") {
            files.insert(format!("data/{}", path.file_name().unwrap().to_string_lossy()), std::fs::read_to_string(&path)?);
        }
    }
    if files.is_empty() {
        bail!("No .sql files in {}", dir.display());
    }
    let api = Api { http: Client::new(), base: client.base_url().trim_end_matches('/').to_string(), project_ref, token: client.bearer_token()? };
    println!("Restoring {} file(s) into {project_ref}...", files.len());
    let result = api.post("/data/restore", &json!({ "files": files }), "restore data")?;
    println!("✅ {} statements", result["statements"].as_u64().unwrap_or(0));
    Ok(())
}

fn deploy_functions(dir: &Path, client: &PicusClient, project_ref: &str, personal_token: Option<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut function_dirs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("picus.toml").exists())
        .collect();
    function_dirs.sort();
    if function_dirs.is_empty() {
        return Ok(());
    }
    println!("\nFunctions ({}):", function_dirs.len());
    for function_dir in function_dirs {
        crate::deploy_function_dir(
            &function_dir,
            client.base_url(),
            Some(project_ref.to_string()),
            None,
            None,
            None,
            None,
            personal_token.clone(),
        )
        .with_context(|| format!("Failed to deploy function {}", function_dir.display()))?;
    }
    Ok(())
}

/// `automations/<id>.json` → `workflow_definition` records, upserted by `id`.
fn restore_automations(api: &Api, dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(());
    }
    println!("\nAutomations:");
    for path in files {
        let row: Value = serde_json::from_str(&std::fs::read_to_string(&path)?).with_context(|| format!("Invalid JSON in {}", path.display()))?;
        let id = row["id"].as_str().unwrap_or_default().to_string();
        let name = row["workflow_name"].as_str().unwrap_or("(unnamed)").to_string();
        let body = json!({ "data": row });
        if !id.is_empty() {
            let res = api
                .http
                .patch(api.url(&format!("/entities/workflow_definition/records/{id}")))
                .bearer_auth(&api.token)
                .json(&body)
                .send()?;
            if res.status().is_success() {
                println!("  = {name}");
                continue;
            }
            if res.status() != reqwest::StatusCode::NOT_FOUND {
                bail!("update automation '{name}' failed ({}): {}", res.status(), res.text().unwrap_or_default());
            }
        }
        api.post("/entities/workflow_definition/records", &body, &format!("create automation '{name}'"))?;
        println!("  + {name}");
    }
    Ok(())
}

/// `ai_agent/agents/<handle>.yaml` → `ai_agent` records (created via the API so the
/// `after_create` hook provisions the agent principal). Skips handles that already exist;
/// `parent_handle` resolves to `parent_agent`, parents first.
fn restore_ai_agents(api: &Api, dir: &Path, bound_user_id: Option<&str>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    let mut pending: Vec<(String, Value)> = Vec::new();
    for path in &paths {
        let value = yaml_to_json(&std::fs::read_to_string(path)?, &path.display().to_string())?;
        let handle = value["handle"].as_str().with_context(|| format!("{}: missing 'handle'", path.display()))?.to_string();
        pending.push((handle, value));
    }
    if pending.is_empty() {
        return Ok(());
    }
    println!("\nAI agents:");

    let existing = api.send(api.http.get(api.url("/entities/ai_agent/records?limit=1000")), "list ai_agent records")?;
    let mut resolved: HashMap<String, String> = existing
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["data"]["handle"].as_str()?.to_string(), row["id"].as_str()?.to_string())))
        .collect();
    pending.retain(|(handle, _)| !resolved.contains_key(handle));
    if pending.is_empty() {
        println!("  (all present)");
        return Ok(());
    }
    let bound_user_id = bound_user_id.context("--bound-user-id is required to create AI agents")?;

    while !pending.is_empty() {
        let before = pending.len();
        let mut waiting = Vec::new();
        for (handle, mut value) in pending {
            let parent = value["parent_handle"].as_str().map(str::to_string);
            if parent.as_ref().is_some_and(|p| !resolved.contains_key(p)) {
                waiting.push((handle, value));
                continue;
            }
            let obj = value.as_object_mut().with_context(|| format!("ai_agent '{handle}' is not a YAML object"))?;
            obj.remove("parent_handle");
            if let Some(parent) = parent {
                obj.insert("parent_agent".to_string(), json!(resolved[&parent]));
            }
            obj.insert("bound_user_id".to_string(), json!(bound_user_id));
            let created = api.post("/entities/ai_agent/records", &json!({ "data": value }), &format!("create ai_agent '{handle}'"))?;
            println!("  + {handle}");
            resolved.insert(handle, created["id"].as_str().unwrap_or_default().to_string());
        }
        pending = waiting;
        if pending.len() == before {
            let handles: Vec<&str> = pending.iter().map(|(h, _)| h.as_str()).collect();
            bail!("Unresolvable parent_handle for: {}", handles.join(", "));
        }
    }
    Ok(())
}
