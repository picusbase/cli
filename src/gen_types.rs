//! `picusbase gen types` — TypeScript from `doctypes/`: one interface per entity, an `Entities`
//! map for typed record calls, and `ENTITIES` metadata (labels, field types, options, relation
//! targets, list/form layout from `views.yaml`) that generic list/form components render from.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::Path;

fn pascal_case(code: &str) -> String {
    code.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}

fn ts_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn options(field: &Value) -> Vec<String> {
    field["predefined_values"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect()
}

fn ts_type(field: &Value) -> String {
    let union = |opts: &[String]| opts.iter().map(|o| ts_string(o)).collect::<Vec<_>>().join(" | ");
    match field["data_type"].as_str().unwrap_or("text") {
        "number" => "number".into(),
        "boolean" => "boolean".into(),
        "json" => "unknown".into(),
        "select" => {
            let opts = options(field);
            if opts.is_empty() { "string".into() } else { union(&opts) }
        }
        "multi_select" => {
            let opts = options(field);
            if opts.is_empty() { "string[]".into() } else { format!("Array<{}>", union(&opts)) }
        }
        _ => "string".into(),
    }
}

fn read_yaml(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text).with_context(|| format!("Invalid YAML in {}", path.display()))?;
    Ok(serde_json::to_value(value)?)
}

pub fn run(bundle_dir: &Path, out: &Path) -> Result<()> {
    let doctypes_dir = bundle_dir.join("doctypes");
    let mut codes: Vec<String> = std::fs::read_dir(&doctypes_dir)
        .with_context(|| format!("Failed to read {}", doctypes_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("doctype.yaml").is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    codes.sort();

    let mut interfaces = String::new();
    let mut entity_map = String::new();
    let mut meta = serde_json::Map::new();

    for code in &codes {
        let doctype = read_yaml(&doctypes_dir.join(code).join("doctype.yaml"))?;
        let views_path = doctypes_dir.join(code).join("views.yaml");
        let views = if views_path.is_file() { read_yaml(&views_path)? } else { Value::Null };

        let mut fields: Vec<Value> = doctype["fields"].as_array().cloned().unwrap_or_default();
        fields.sort_by_key(|f| f["display_order"].as_i64().unwrap_or(0));
        let name = pascal_case(code);

        let label = doctype["entity_name"].as_str().unwrap_or(code);
        let _ = writeln!(interfaces, "/** {label} (`{code}`) */\nexport interface {name} {{");
        for field in &fields {
            let field_code = field["field_code"].as_str().unwrap_or_default();
            let ty = ts_type(field);
            let doc = field["field_name"].as_str().unwrap_or(field_code);
            if field["is_required"].as_bool().unwrap_or(false) {
                let _ = writeln!(interfaces, "  /** {doc} */\n  {field_code}: {ty};");
            } else {
                let _ = writeln!(interfaces, "  /** {doc} */\n  {field_code}?: {ty} | null;");
            }
        }
        interfaces.push_str("}\n\n");
        let _ = writeln!(entity_map, "  {code}: {name};");

        let field_codes: Vec<&str> = fields.iter().filter_map(|f| f["field_code"].as_str()).collect();
        let list = views["list_view"]["columns"].as_array().cloned().unwrap_or_else(|| field_codes.iter().map(|c| json!(c)).collect());
        let form = views["form_view"]["sections"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![json!({ "title": label, "fields": field_codes })]);
        let field_meta: Vec<Value> = fields
            .iter()
            .map(|f| {
                let mut m = json!({
                    "code": f["field_code"],
                    "label": f["field_name"],
                    "type": f["data_type"],
                    "required": f["is_required"].as_bool().unwrap_or(false),
                });
                let opts = options(f);
                if !opts.is_empty() {
                    m["options"] = json!(opts);
                }
                if let Some(target) = f["validation_rules"]["target_entity_code"].as_str() {
                    m["target"] = json!(target);
                }
                m
            })
            .collect();
        meta.insert(
            code.clone(),
            json!({
                "code": code,
                "name": label,
                "modules": doctype["modules"].as_array().cloned().unwrap_or_default(),
                "fields": field_meta,
                "list": list,
                "form": form,
            }),
        );
    }

    let mut ts = String::from(
        "// AUTO-GENERATED by `picusbase gen types` from doctypes/ — do not edit; re-run after changing a doctype.\n\n\
         export type DataType = 'text' | 'number' | 'boolean' | 'date' | 'color' | 'image' | 'select' | 'multi_select' | 'relation' | 'json';\n\n\
         export interface FieldMeta {\n  code: string;\n  label: string;\n  type: DataType;\n  required: boolean;\n  options?: readonly string[];\n  /** Target entity of a `relation` field. */\n  target?: string;\n}\n\n\
         export interface EntityMeta {\n  code: string;\n  name: string;\n  modules: readonly string[];\n  fields: readonly FieldMeta[];\n  /** Columns of the default list view. */\n  list: readonly string[];\n  form: readonly { title: string; fields: readonly string[] }[];\n}\n\n",
    );
    ts.push_str(&interfaces);
    let _ = writeln!(ts, "export interface Entities {{\n{entity_map}}}\n\nexport type EntityCode = keyof Entities;\n");
    let _ = writeln!(
        ts,
        "export const ENTITIES: Record<EntityCode, EntityMeta> = {} as const;",
        serde_json::to_string_pretty(&Value::Object(meta))?
    );

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, ts).with_context(|| format!("Failed to write {}", out.display()))?;
    println!("✅ {} entities → {}", codes.len(), out.display());
    Ok(())
}
