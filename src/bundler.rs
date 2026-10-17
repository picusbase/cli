//! Phase 4 slice B + C — bundlers for the JavaScript and Python lanes.
//!
//! The CLI does not run a JavaScript bundler (esbuild/webpack). Authors are
//! expected to produce a source folder whose layout matches the contract;
//! this module:
//!
//! 1. Validates the layout (required files exist, banned constructs absent).
//! 2. Packages the folder into a deterministic tar.gz that the control
//!    plane can hash and store as the publish artifact.
//!
//! Authoring layout is normalized before packaging: JavaScript authoring must
//! provide `src/index.mjs` and Python authoring must provide
//! `src/handler.py`.
//! Output formats are locked by `docs-platform/01-architecture/04-compute-architecture.md`:
//! - `js-bundle-v1`     → tar.gz with `manifest.json`, `payload/index.mjs`,
//!   optional `assets/`.
//! - `py-source-bundle-v1` → tar.gz with `manifest.json`,
//!   `payload/handler.py` (stdlib only — the CPython gVisor shim installs no
//!   third-party packages), optional `assets/`.

use anyhow::{anyhow, bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use regex::Regex;
use serde::Serialize;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use walkdir::WalkDir;

pub const PYTHON_V1_RUNTIME_CLASS: &str = "python-v1";

pub fn is_supported_python_runtime_class(runtime_class: &str) -> bool {
    matches!(runtime_class, PYTHON_V1_RUNTIME_CLASS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleKind {
    JsBundleV1,
    PySourceBundleV1,
}

impl BundleKind {
    #[allow(dead_code)]
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "js" | "javascript" | "js-bundle-v1" | "javascript-v1" => {
                Ok(BundleKind::JsBundleV1)
            }
            "python" | "py" | "py-source-bundle-v1" | "python-v1" => {
                Ok(BundleKind::PySourceBundleV1)
            }
            other => Err(anyhow!(
                "Unknown bundle kind '{}'. Expected one of: js, python (see docs-platform/01-architecture/04-compute-architecture.md).",
                other
            )),
        }
    }

    pub fn runtime_class(self) -> &'static str {
        match self {
            BundleKind::JsBundleV1 => "javascript-v1",
            BundleKind::PySourceBundleV1 => PYTHON_V1_RUNTIME_CLASS,
        }
    }

    pub fn language(self) -> &'static str {
        match self {
            BundleKind::JsBundleV1 => "javascript",
            BundleKind::PySourceBundleV1 => "python",
        }
    }

    pub fn payload_filename(self) -> &'static str {
        match self {
            BundleKind::JsBundleV1 => "index.mjs",
            BundleKind::PySourceBundleV1 => "handler.py",
        }
    }
}

/// Result of packaging — caller uploads `archive_path` and optionally
/// records `manifest_checksum` / `archive_size`.
#[derive(Debug, Clone)]
pub struct BundleArtifact {
    pub archive_path: PathBuf,
    pub archive_size: u64,
    pub validated_files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BundleManifest {
    schema_version: &'static str,
    bundle_kind: &'static str,
    runtime_class: String,
    language: &'static str,
    entrypoint: String,
    allowed_capabilities: Vec<String>,
    files: Vec<String>,
}

/// Public entry: validate the source layout, then write a deterministic
/// tar.gz archive to `out_path`.
#[allow(dead_code)]
pub fn package_bundle(kind: BundleKind, src_dir: &Path, out_path: &Path) -> Result<BundleArtifact> {
    package_bundle_with_capabilities(kind, src_dir, out_path, &[])
}

pub fn package_bundle_with_capabilities(
    kind: BundleKind,
    src_dir: &Path,
    out_path: &Path,
    allowed_capabilities: &[String],
) -> Result<BundleArtifact> {
    package_bundle_with_capabilities_for_runtime_class(
        kind,
        src_dir,
        out_path,
        allowed_capabilities,
        kind.runtime_class(),
    )
}

pub fn package_bundle_with_capabilities_for_runtime_class(
    kind: BundleKind,
    src_dir: &Path,
    out_path: &Path,
    allowed_capabilities: &[String],
    runtime_class: &str,
) -> Result<BundleArtifact> {
    validate_runtime_class_for_bundle(kind, runtime_class)?;
    if !src_dir.is_dir() {
        bail!(
            "Bundle source directory does not exist: {}",
            src_dir.display()
        );
    }

    let staged = stage_bundle_source(kind, src_dir)?;
    let normalized_src = staged.path();

    let mut validated = match kind {
        BundleKind::JsBundleV1 => validate_js_bundle(normalized_src)?,
        BundleKind::PySourceBundleV1 => validate_py_source_bundle(normalized_src)?,
    };
    validated.sort();

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create output directory {}", parent.display())
            })?;
        }
    }

    let manifest_bytes = build_bundle_manifest(kind, runtime_class, &validated, allowed_capabilities)?;
    let archive_size = write_targz(normalized_src, out_path, &manifest_bytes)?;
    Ok(BundleArtifact {
        archive_path: out_path.to_path_buf(),
        archive_size,
        validated_files: validated,
    })
}

fn validate_runtime_class_for_bundle(kind: BundleKind, runtime_class: &str) -> Result<()> {
    match kind {
        BundleKind::JsBundleV1 if runtime_class == kind.runtime_class() => Ok(()),
        BundleKind::PySourceBundleV1 if is_supported_python_runtime_class(runtime_class) => Ok(()),
        BundleKind::JsBundleV1 => bail!(
            "js-bundle-v1 only supports runtime_class '{}', got '{}'",
            kind.runtime_class(),
            runtime_class
        ),
        BundleKind::PySourceBundleV1 => bail!(
            "py-source-bundle-v1 only supports runtime_class '{}', got '{}'",
            PYTHON_V1_RUNTIME_CLASS,
            runtime_class
        ),
    }
}

fn stage_bundle_source(kind: BundleKind, src_dir: &Path) -> Result<TempDir> {
    let staged = tempfile::Builder::new()
        .prefix("picus-cli-bundle-")
        .tempdir()
        .context("Failed to create temporary staging directory for bundle packaging")?;

    let staged_root = staged.path();
    let payload_dir = staged_root.join("payload");
    std::fs::create_dir_all(&payload_dir)
        .with_context(|| format!("Failed to create {}", payload_dir.display()))?;

    match kind {
        BundleKind::JsBundleV1 => {
            let source_root = resolve_javascript_source(src_dir)?;
            copy_dir_contents(&source_root, &payload_dir)?;

            let assets_dir = src_dir.join("assets");
            if assets_dir.is_dir() {
                copy_dir_contents(&assets_dir, &staged_root.join("assets"))?;
            }
        }
        BundleKind::PySourceBundleV1 => {
            let source_root = resolve_python_source(src_dir)?;
            copy_dir_contents(&source_root, &payload_dir)?;

            let assets_dir = src_dir.join("assets");
            if assets_dir.is_dir() {
                copy_dir_contents(&assets_dir, &staged_root.join("assets"))?;
            }
        }
    }

    Ok(staged)
}

fn copy_dir_contents(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("Failed to create {}", to.display()))?;

    for entry in WalkDir::new(from).min_depth(1) {
        let entry = entry.with_context(|| format!("Failed to walk {}", from.display()))?;
        let source_path = entry.path();
        let relative = source_path.strip_prefix(from).with_context(|| {
            format!(
                "Failed to relativize {} against {}",
                source_path.display(),
                from.display()
            )
        })?;
        let destination = to.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination)
                .with_context(|| format!("Failed to create {}", destination.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create {}", parent.display()))?;
            }
            std::fs::copy(source_path, &destination).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    source_path.display(),
                    destination.display()
                )
            })?;
        }
    }

    Ok(())
}

fn resolve_javascript_source(src_dir: &Path) -> Result<PathBuf> {
    let src_root = src_dir.join("src");
    if src_root.join("index.mjs").is_file() {
        return Ok(src_root);
    }

    bail!(
        "js-bundle-v1 source must contain src/index.mjs under {}",
        src_dir.display()
    )
}

fn resolve_python_source(src_dir: &Path) -> Result<PathBuf> {
    let src_root = src_dir.join("src");
    if src_root.join("handler.py").is_file() {
        return Ok(src_root);
    }

    bail!(
        "py-source-bundle-v1 source must contain src/handler.py under {}",
        src_dir.display()
    )
}

fn build_bundle_manifest(
    kind: BundleKind,
    runtime_class: &str,
    validated_files: &[String],
    allowed_capabilities: &[String],
) -> Result<Vec<u8>> {
    let manifest = BundleManifest {
        schema_version: "v1",
        bundle_kind: match kind {
            BundleKind::JsBundleV1 => "js-bundle-v1",
            BundleKind::PySourceBundleV1 => "py-source-bundle-v1",
        },
        runtime_class: runtime_class.to_string(),
        language: kind.language(),
        entrypoint: format!("payload/{}", kind.payload_filename()),
        allowed_capabilities: allowed_capabilities.to_vec(),
        files: validated_files.to_vec(),
    };

    serde_json::to_vec(&manifest).context("Failed to serialize bundle manifest.json")
}

// ---------------------------------------------------------------------------
// JS bundle validation
// ---------------------------------------------------------------------------

/// Constructs we hard-reject in `payload/*.mjs`. Each entry is a regex
/// matched line-by-line (after stripping single-line `//` comments).
const JS_BANNED_PATTERNS: &[(&str, &str)] = &[
    ("commonjs_require", r"\brequire\s*\("),
    ("commonjs_module_exports", r"\bmodule\.exports\b"),
    ("dynamic_import", r"\bimport\s*\("),
    ("node_process", r"\bprocess\.(env|exit|cwd|argv)\b"),
    (
        "node_builtin_import",
        r#"from\s+['"](fs|path|os|child_process|net|dns|http|https|crypto|stream|cluster|worker_threads)['"]"#,
    ),
];

pub fn validate_js_bundle(src_dir: &Path) -> Result<Vec<String>> {
    let payload_dir = src_dir.join("payload");
    let entry = payload_dir.join("index.mjs");
    if !entry.is_file() {
        bail!(
            "js-bundle-v1 requires payload/index.mjs at {}",
            entry.display()
        );
    }

    let mut validated = Vec::new();
    let comment_strip = Regex::new(r"//[^\n]*").expect("static regex");

    for entry_res in WalkDir::new(&payload_dir).into_iter() {
        let entry =
            entry_res.with_context(|| format!("Failed to walk {}", payload_dir.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "mjs" && ext != "js" {
            continue;
        }

        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let stripped = comment_strip.replace_all(&source, "");

        for (rule_id, pattern) in JS_BANNED_PATTERNS {
            let re = Regex::new(pattern).expect("static banned pattern");
            if re.is_match(&stripped) {
                bail!(
                    "js-bundle-v1 validation failed in {}: banned construct '{}' (rule {}). \
                     The Picus JS lane targets ES modules only and does not provide Node built-ins.",
                    path.display(),
                    pattern,
                    rule_id
                );
            }
        }

        validated.push(
            path.strip_prefix(src_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
        );
    }

    Ok(validated)
}

// ---------------------------------------------------------------------------
// Python source bundle validation
// ---------------------------------------------------------------------------

pub fn validate_py_source_bundle(src_dir: &Path) -> Result<Vec<String>> {
    let payload_dir = src_dir.join("payload");
    let handler = payload_dir.join("handler.py");
    if !handler.is_file() {
        bail!(
            "py-source-bundle-v1 requires payload/handler.py at {}",
            handler.display()
        );
    }

    Ok(vec![handler
        .strip_prefix(src_dir)
        .unwrap_or(&handler)
        .to_string_lossy()
        .into_owned()])
}

// ---------------------------------------------------------------------------
// native-v1 — compiled native Rust binary bundle.
//
// Unlike JsBundleV1/PySourceBundleV1 (which copy authoring SOURCE into the
// bundle for the guest shim to interpret), a native-v1 bundle carries the
// already-COMPILED binary — `RustAdapter::build_with_runtime_class` builds it
// (cross-compiled for the gVisor node's OS/arch, see language.rs) and hands
// the resulting path here. Reuses the same `write_targz` layout
// (`manifest.json` + `payload/<entrypoint>`) the other lanes use so
// `services/compute-native-shim` (the generic native-v1 guest shim) can
// unpack it identically.
// ---------------------------------------------------------------------------

pub fn package_native_binary_bundle(
    binary_path: &Path,
    out_path: &Path,
    allowed_capabilities: &[String],
) -> Result<BundleArtifact> {
    let staged = tempfile::Builder::new()
        .prefix("picus-cli-native-bundle-")
        .tempdir()
        .context("Failed to create temporary staging directory for native bundle packaging")?;
    let payload_dir = staged.path().join("payload");
    std::fs::create_dir_all(&payload_dir)
        .with_context(|| format!("Failed to create {}", payload_dir.display()))?;
    let dest = payload_dir.join("worker");
    std::fs::copy(binary_path, &dest)
        .with_context(|| format!("Failed to copy built binary {} to {}", binary_path.display(), dest.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output directory {}", parent.display()))?;
        }
    }

    let manifest = serde_json::json!({
        "schema_version": "v1",
        "bundle_kind": "native-bin-v1",
        "runtime_class": "native-v1",
        "language": "rust",
        "entrypoint": "payload/worker",
        "allowed_capabilities": allowed_capabilities,
        "files": ["payload/worker"],
    });
    let manifest_bytes =
        serde_json::to_vec(&manifest).context("Failed to serialize native bundle manifest.json")?;
    let archive_size = write_targz(staged.path(), out_path, &manifest_bytes)?;

    Ok(BundleArtifact {
        archive_path: out_path.to_path_buf(),
        archive_size,
        validated_files: vec!["payload/worker".to_string()],
    })
}

// ---------------------------------------------------------------------------
// Deterministic tar.gz writer
// ---------------------------------------------------------------------------

fn write_targz(src_dir: &Path, out_path: &Path, manifest_bytes: &[u8]) -> Result<u64> {
    let file = File::create(out_path)
        .with_context(|| format!("Failed to create archive {}", out_path.display()))?;
    let writer = BufWriter::new(file);
    let encoder = GzEncoder::new(writer, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);

    append_virtual_file(&mut builder, Path::new("manifest.json"), manifest_bytes)?;

    // Walk in sorted order so byte-equal sources produce byte-equal archives.
    // Only payload/ and assets/ belong to the runtime bundle; keep human
    // metadata like README.md out of the archive so worker-side unpackers do
    // not reject the bundle.
    let mut entries: Vec<_> = WalkDir::new(src_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| include_bundle_entry(src_dir, entry.path()))
        .collect();
    entries.sort_by(|a, b| a.path().cmp(b.path()));

    for entry in entries {
        let path = entry.path();
        let rel = path
            .strip_prefix(src_dir)
            .with_context(|| format!("Failed to relativize {}", path.display()))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            builder.append_dir(rel, path)?;
        } else if entry.file_type().is_file() {
            let mut f =
                File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
            builder.append_file(rel, &mut f)?;
        }
        // Symlinks are skipped intentionally — they could escape the bundle.
    }

    let encoder = builder.into_inner()?;
    let writer = encoder.finish()?;
    writer.into_inner()?.sync_all().ok();

    let metadata = std::fs::metadata(out_path)?;
    Ok(metadata.len())
}

fn include_bundle_entry(src_dir: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(src_dir) else {
        return false;
    };

    let Some(first) = rel.components().next() else {
        return false;
    };

    matches!(first.as_os_str().to_str(), Some("payload") | Some("assets"))
}

fn append_virtual_file(
    builder: &mut tar::Builder<GzEncoder<BufWriter<File>>>,
    path: &Path,
    bytes: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    builder
        .append_data(&mut header, path, std::io::Cursor::new(bytes))
        .with_context(|| format!("Failed to append virtual file {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(file: &Path, content: &str) {
        if let Some(p) = file.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        let mut f = std::fs::File::create(file).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn js_bundle_accepts_clean_esm() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-js-ok");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("src/index.mjs"),
            "export default function handler(req) { return { ok: true }; }\n",
        );
        write(&dir.join("README.md"), "human metadata only\n");
        let out = dir.join("bundle.tar.gz");
        let artifact = package_bundle(BundleKind::JsBundleV1, &dir, &out).unwrap();
        assert!(artifact.archive_size > 0);
        assert!(out.exists());
        let names = archive_paths(&out);
        assert!(names.iter().any(|name| name == "payload/index.mjs"));
        assert!(!names.iter().any(|name| name == "README.md"));
    }

    #[test]
    fn js_bundle_rejects_require_call() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-js-cjs");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("src/index.mjs"),
            "const fs = require('fs');\nexport default () => fs;\n",
        );
        let err =
            package_bundle(BundleKind::JsBundleV1, &dir, &dir.join("bundle.tar.gz")).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("commonjs_require") || msg.contains("require"));
    }

    #[test]
    fn js_bundle_rejects_node_builtin_import() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-js-node");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("src/index.mjs"),
            "import { readFile } from 'fs';\nexport default () => readFile;\n",
        );
        let err =
            package_bundle(BundleKind::JsBundleV1, &dir, &dir.join("bundle.tar.gz")).unwrap_err();
        assert!(format!("{}", err).contains("node_builtin_import"));
    }

    #[test]
    fn py_bundle_accepts_handler_only() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-py-handler");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("src/handler.py"),
            "def handler(req):\n    return {\"ok\": True}\n",
        );
        let out = dir.join("bundle.tar.gz");
        let artifact = package_bundle(BundleKind::PySourceBundleV1, &dir, &out).unwrap();
        assert!(artifact.archive_size > 0);
        let names = archive_paths(&out);
        assert!(names.iter().any(|name| name == "manifest.json"));
        assert!(names.iter().any(|name| name == "payload/handler.py"));
    }

    #[test]
    fn js_bundle_rejects_legacy_payload_layout() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-js-payload-legacy");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("payload/index.mjs"),
            "export default function handler(req) { return { ok: true }; }\n",
        );
        let err =
            package_bundle(BundleKind::JsBundleV1, &dir, &dir.join("bundle.tar.gz")).unwrap_err();
        assert!(format!("{}", err).contains("src/index.mjs"));
    }

    #[test]
    fn py_bundle_rejects_legacy_payload_layout() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-py-payload-legacy");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("payload/handler.py"),
            "def handler(req):\n    return {\"ok\": True}\n",
        );
        let err = package_bundle(
            BundleKind::PySourceBundleV1,
            &dir,
            &dir.join("bundle.tar.gz"),
        )
        .unwrap_err();
        assert!(format!("{}", err).contains("src/handler.py"));
    }

    #[test]
    fn py_bundle_rejects_legacy_root_handler_layout() {
        let dir = std::env::temp_dir().join("picus-cli-bundle-py-root-legacy");
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir.join("handler.py"),
            "def handler(req):\n    return {\"ok\": True}\n",
        );
        let err = package_bundle(
            BundleKind::PySourceBundleV1,
            &dir,
            &dir.join("bundle.tar.gz"),
        )
        .unwrap_err();
        assert!(format!("{}", err).contains("src/handler.py"));
    }

    fn archive_paths(archive: &Path) -> Vec<String> {
        let file = std::fs::File::open(archive).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}
