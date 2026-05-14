pub(crate) mod bot_bridge;
pub(crate) mod gateway;

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const BOT_GATEWAY_EXTENSION_ID: &str = "bot-gateway";
const BOT_GATEWAY_EXTENSION_NAME: &str = "Bot";
const BOT_GATEWAY_EXTENSION_DESCRIPTION: &str =
    "Connect Codex to im platforms through the built-in Bot plugin.";
const BOT_GATEWAY_EXTENSION_VERSION: &str = "1.0.0";
const BOT_GATEWAY_BUNDLED_PACKAGE_FILE: &str = "bot-gateway-1.0.0.tar.gz";
const BOT_GATEWAY_ENTRY_ENV: &str = "CODEXL_BUILTIN_BOT_GATEWAY_ENTRY";
const BOT_GATEWAY_PACKAGE_ENV: &str = "CODEXL_BUILTIN_BOT_GATEWAY_PACKAGE";
const BOT_GATEWAY_PACKAGE_URL_ENV: &str = "CODEXL_BUILTIN_BOT_GATEWAY_PACKAGE_URL";
const BOT_GATEWAY_UPDATE_MANIFEST_URL_ENV: &str = "CODEXL_BUILTIN_BOT_GATEWAY_UPDATE_MANIFEST_URL";
const NEXT_AI_GATEWAY_EXTENSION_ID: &str = "next-ai-gateway";
const NEXT_AI_GATEWAY_EXTENSION_NAME: &str = "NeXT AI Gateway";
const NEXT_AI_GATEWAY_EXTENSION_DESCRIPTION: &str =
    "Use the built-in Gateway to convert other protocol interfaces for Codex.";
const NEXT_AI_GATEWAY_EXTENSION_VERSION: &str = "1.0.0";
const NEXT_AI_GATEWAY_BUNDLED_PACKAGE_FILE: &str = "next-ai-gateway-1.0.0.tar.gz";
const NEXT_AI_GATEWAY_ENTRY_ENV: &str = "CODEXL_BUILTIN_NEXT_AI_GATEWAY_ENTRY";
const NEXT_AI_GATEWAY_PACKAGE_ENV: &str = "CODEXL_BUILTIN_NEXT_AI_GATEWAY_PACKAGE";
const NEXT_AI_GATEWAY_PACKAGE_URL_ENV: &str = "CODEXL_BUILTIN_NEXT_AI_GATEWAY_PACKAGE_URL";
const NEXT_AI_GATEWAY_UPDATE_MANIFEST_URL_ENV: &str =
    "CODEXL_BUILTIN_NEXT_AI_GATEWAY_UPDATE_MANIFEST_URL";
const CODEXL_HOME_ENV: &str = "CODEXL_HOME";
const NODE_PATH_ENV: &str = "CODEXL_NODE_PATH";
const NODE_DIST_BASE_ENV: &str = "CODEXL_NODE_DIST_BASE_URL";
const NODE_RELEASE_LINE: &str = "latest-v22.x";
const DEFAULT_NODE_DIST_BASE: &str = "https://nodejs.org/dist";
const MIN_NODE_MAJOR: u32 = 20;

#[derive(Debug, Clone, Copy)]
struct BuiltinNodeExtensionSpec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    version: &'static str,
    bundled_package_file: &'static str,
    entry: &'static str,
    entry_env: &'static str,
    package_env: &'static str,
    package_url_env: &'static str,
    update_manifest_url_env: &'static str,
}

const BOT_GATEWAY_SPEC: BuiltinNodeExtensionSpec = BuiltinNodeExtensionSpec {
    id: BOT_GATEWAY_EXTENSION_ID,
    name: BOT_GATEWAY_EXTENSION_NAME,
    description: BOT_GATEWAY_EXTENSION_DESCRIPTION,
    version: BOT_GATEWAY_EXTENSION_VERSION,
    bundled_package_file: BOT_GATEWAY_BUNDLED_PACKAGE_FILE,
    entry: "stdio/stdio.js",
    entry_env: BOT_GATEWAY_ENTRY_ENV,
    package_env: BOT_GATEWAY_PACKAGE_ENV,
    package_url_env: BOT_GATEWAY_PACKAGE_URL_ENV,
    update_manifest_url_env: BOT_GATEWAY_UPDATE_MANIFEST_URL_ENV,
};

const NEXT_AI_GATEWAY_SPEC: BuiltinNodeExtensionSpec = BuiltinNodeExtensionSpec {
    id: NEXT_AI_GATEWAY_EXTENSION_ID,
    name: NEXT_AI_GATEWAY_EXTENSION_NAME,
    description: NEXT_AI_GATEWAY_EXTENSION_DESCRIPTION,
    version: NEXT_AI_GATEWAY_EXTENSION_VERSION,
    bundled_package_file: NEXT_AI_GATEWAY_BUNDLED_PACKAGE_FILE,
    entry: "gateway/start.js",
    entry_env: NEXT_AI_GATEWAY_ENTRY_ENV,
    package_env: NEXT_AI_GATEWAY_PACKAGE_ENV,
    package_url_env: NEXT_AI_GATEWAY_PACKAGE_URL_ENV,
    update_manifest_url_env: NEXT_AI_GATEWAY_UPDATE_MANIFEST_URL_ENV,
};

#[derive(Debug, Clone)]
pub struct BuiltinNodeExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root_dir: PathBuf,
    pub entry_path: PathBuf,
    pub node: NodeRuntime,
}

#[derive(Debug, Clone)]
pub struct NodeRuntime {
    pub executable: PathBuf,
    pub source: RuntimeSource,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSource {
    Explicit,
    System,
    Managed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinExtensionStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub runtime: RuntimeStatus,
    pub entry_path: String,
    pub ready: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub kind: String,
    pub executable: String,
    pub source: String,
    pub version: String,
    pub installed: bool,
}

#[derive(Debug, Clone)]
struct InstalledPluginPackage {
    manifest: PluginManifest,
    root_dir: PathBuf,
    entry_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct PluginManifest {
    id: String,
    name: String,
    description: String,
    version: String,
    runtime: String,
    entry: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemotePluginRelease {
    id: Option<String>,
    version: String,
    url: Option<String>,
    #[serde(alias = "package_url")]
    package_url: Option<String>,
    #[serde(alias = "archive_url")]
    archive_url: Option<String>,
}

impl PluginManifest {
    fn fallback(spec: BuiltinNodeExtensionSpec) -> Self {
        Self {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            version: spec.version.to_string(),
            runtime: "nodejs".to_string(),
            entry: spec.entry.to_string(),
        }
    }
}

impl RemotePluginRelease {
    fn package_url(&self) -> Result<&str, String> {
        self.package_url
            .as_deref()
            .or(self.archive_url.as_deref())
            .or(self.url.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Bot plugin update manifest does not include a package URL".to_string())
    }
}

pub fn builtin_bot_gateway_status() -> BuiltinExtensionStatus {
    builtin_node_extension_status(BOT_GATEWAY_SPEC)
}

pub fn builtin_next_ai_gateway_status() -> BuiltinExtensionStatus {
    builtin_node_extension_status(NEXT_AI_GATEWAY_SPEC)
}

fn builtin_node_extension_status(spec: BuiltinNodeExtensionSpec) -> BuiltinExtensionStatus {
    let installed_result = resolve_installed_plugin_package(spec);
    let installed = installed_result
        .as_ref()
        .ok()
        .and_then(|value| value.as_ref());
    let runtime = resolve_existing_node_runtime();
    let bundled_available = bundled_plugin_package_path(spec).is_ok();
    let package_available = installed.is_some() || bundled_available;
    let ready = package_available && runtime.is_some();
    let message = match (&installed_result, package_available, &runtime) {
        (Err(err), false, _) => err.clone(),
        (_, false, _) => format!("built-in {} plugin package was not found", spec.name),
        (_, true, None) => format!("Node.js {}+ is required", MIN_NODE_MAJOR),
        (_, true, Some(_)) => "ready".to_string(),
    };
    let manifest = installed.map(|package| &package.manifest);

    BuiltinExtensionStatus {
        id: manifest
            .map(|manifest| manifest.id.clone())
            .unwrap_or_else(|| spec.id.to_string()),
        name: manifest
            .map(|manifest| manifest.name.clone())
            .unwrap_or_else(|| spec.name.to_string()),
        description: manifest
            .map(|manifest| manifest.description.clone())
            .unwrap_or_else(|| spec.description.to_string()),
        version: manifest
            .map(|manifest| manifest.version.clone())
            .unwrap_or_else(|| spec.version.to_string()),
        runtime: runtime_status(runtime.as_ref()),
        entry_path: installed
            .map(|package| package.entry_path.to_string_lossy().to_string())
            .unwrap_or_default(),
        ready,
        message,
    }
}

pub fn prepare_builtin_bot_gateway() -> Result<BuiltinExtensionStatus, String> {
    prepare_builtin_node_extension(BOT_GATEWAY_SPEC)
}

pub fn prepare_builtin_next_ai_gateway() -> Result<BuiltinExtensionStatus, String> {
    prepare_builtin_node_extension(NEXT_AI_GATEWAY_SPEC)
}

pub fn prepare_builtin_extensions_runtime() -> Result<RuntimeStatus, String> {
    let node = ensure_node_runtime()?;
    Ok(runtime_status(Some(&node)))
}

fn prepare_builtin_node_extension(
    spec: BuiltinNodeExtensionSpec,
) -> Result<BuiltinExtensionStatus, String> {
    let extension = resolve_builtin_node_extension(spec)?;
    Ok(BuiltinExtensionStatus {
        id: extension.id,
        name: extension.name,
        version: extension.version,
        description: spec.description.to_string(),
        runtime: runtime_status(Some(&extension.node)),
        entry_path: extension.entry_path.to_string_lossy().to_string(),
        ready: true,
        message: "ready".to_string(),
    })
}

pub fn resolve_builtin_bot_gateway_extension() -> Result<BuiltinNodeExtension, String> {
    resolve_builtin_node_extension(BOT_GATEWAY_SPEC)
}

pub fn resolve_builtin_next_ai_gateway_extension() -> Result<BuiltinNodeExtension, String> {
    resolve_builtin_node_extension(NEXT_AI_GATEWAY_SPEC)
}

fn resolve_builtin_node_extension(
    spec: BuiltinNodeExtensionSpec,
) -> Result<BuiltinNodeExtension, String> {
    let installed = resolve_builtin_plugin_installation(spec)?;
    let node = ensure_node_runtime()?;

    Ok(BuiltinNodeExtension {
        id: installed.manifest.id,
        name: installed.manifest.name,
        version: installed.manifest.version,
        root_dir: installed.root_dir,
        entry_path: installed.entry_path,
        node,
    })
}

fn runtime_status(runtime: Option<&NodeRuntime>) -> RuntimeStatus {
    RuntimeStatus {
        kind: "nodejs".to_string(),
        executable: runtime
            .map(|runtime| runtime.executable.to_string_lossy().to_string())
            .unwrap_or_default(),
        source: runtime
            .map(|runtime| runtime.source.as_str().to_string())
            .unwrap_or_default(),
        version: runtime
            .map(|runtime| runtime.version.clone())
            .unwrap_or_default(),
        installed: runtime.is_some(),
    }
}

fn resolve_builtin_plugin_installation(
    spec: BuiltinNodeExtensionSpec,
) -> Result<InstalledPluginPackage, String> {
    if let Some(path) = env_path(spec.entry_env) {
        return resolve_entry_override(path, spec);
    }

    ensure_builtin_plugin_package(spec)
}

fn resolve_entry_override(
    path: PathBuf,
    spec: BuiltinNodeExtensionSpec,
) -> Result<InstalledPluginPackage, String> {
    let entry_path = validate_plugin_entry(path, spec)?;
    let root_dir = plugin_root_from_entry(&entry_path)?;
    let manifest =
        read_plugin_manifest(&root_dir).unwrap_or_else(|_| PluginManifest::fallback(spec));

    Ok(InstalledPluginPackage {
        manifest,
        root_dir,
        entry_path,
    })
}

fn ensure_builtin_plugin_package(
    spec: BuiltinNodeExtensionSpec,
) -> Result<InstalledPluginPackage, String> {
    if let Some(package_path) = env_path(spec.package_env) {
        install_plugin_package(spec, &package_path)?;
        return resolve_installed_plugin_package(spec)?
            .ok_or_else(|| format!("{} plugin package was not installed", spec.name));
    }

    let installed = resolve_installed_plugin_package(spec).unwrap_or(None);
    let remote_error = match try_remote_plugin_update(spec, installed.as_ref()) {
        Ok(Some(remote_package)) => return Ok(remote_package),
        Ok(None) => None,
        Err(err) => {
            eprintln!("failed to update {} plugin package: {}", spec.name, err);
            Some(err)
        }
    };

    if let Some(package) = installed.as_ref() {
        if !version_is_newer(spec.version, &package.manifest.version) {
            return Ok(package.clone());
        }
    }

    match bundled_plugin_package_path(spec).and_then(|path| install_plugin_package(spec, &path)) {
        Ok(package) => Ok(package),
        Err(err) => installed.ok_or_else(|| remote_error.unwrap_or(err)),
    }
}

fn try_remote_plugin_update(
    spec: BuiltinNodeExtensionSpec,
    installed: Option<&InstalledPluginPackage>,
) -> Result<Option<InstalledPluginPackage>, String> {
    if let Some(manifest_url) = env_string(spec.update_manifest_url_env) {
        let release = http_get_string(&manifest_url)
            .and_then(|content| {
                serde_json::from_str::<RemotePluginRelease>(&content).map_err(|err| {
                    format!(
                        "failed to parse {} plugin update manifest: {}",
                        spec.name, err
                    )
                })
            })
            .and_then(|release| {
                validate_remote_release(spec, &release)?;
                Ok(release)
            });

        match release {
            Ok(release) => {
                let current_version = current_plugin_package_version(spec, installed);
                if current_version
                    .map(|version| !version_is_newer(&release.version, version))
                    .unwrap_or(false)
                {
                    return Ok(None);
                }
                let package_url = release.package_url()?;
                let archive_path =
                    download_extension_package(spec, package_url, Some(&release.version))?;
                let package = install_plugin_package(spec, &archive_path)?;
                if package.manifest.version != release.version {
                    return Err(format!(
                        "{} plugin package version {} did not match update manifest version {}",
                        spec.name, package.manifest.version, release.version
                    ));
                }
                Ok(Some(package))
            }
            Err(err) if installed.is_some() => {
                eprintln!("failed to check {} plugin update: {}", spec.name, err);
                Ok(None)
            }
            Err(err) => Err(err),
        }
    } else if let Some(package_url) = env_string(spec.package_url_env) {
        match download_extension_package(spec, &package_url, None)
            .and_then(|archive_path| install_plugin_package(spec, &archive_path))
        {
            Ok(package) => {
                if current_plugin_package_version(spec, installed)
                    .map(|version| version_is_newer(version, &package.manifest.version))
                    .unwrap_or(false)
                {
                    Ok(None)
                } else {
                    Ok(Some(package))
                }
            }
            Err(err) if installed.is_some() => {
                eprintln!("failed to download {} plugin package: {}", spec.name, err);
                Ok(None)
            }
            Err(err) => Err(err),
        }
    } else {
        Ok(None)
    }
}

fn current_plugin_package_version(
    spec: BuiltinNodeExtensionSpec,
    installed: Option<&InstalledPluginPackage>,
) -> Option<&str> {
    installed
        .map(|package| package.manifest.version.as_str())
        .or_else(|| bundled_plugin_package_path(spec).ok().map(|_| spec.version))
}

fn install_plugin_package(
    spec: BuiltinNodeExtensionSpec,
    archive_path: &Path,
) -> Result<InstalledPluginPackage, String> {
    if !archive_path.is_file() {
        return Err(format!(
            "{} plugin package not found: {}",
            spec.name,
            archive_path.to_string_lossy()
        ));
    }

    let install_root = extension_install_root(spec.id);
    fs::create_dir_all(&install_root).map_err(|err| err.to_string())?;
    let extract_dir =
        install_root.join(format!(".extract-{}-{}", std::process::id(), unix_millis()));
    fs::create_dir_all(&extract_dir).map_err(|err| err.to_string())?;

    extract_plugin_package_archive(spec, archive_path, &extract_dir)?;
    let extracted_root = extracted_plugin_root(&extract_dir)?;
    let manifest = read_plugin_manifest(&extracted_root)?;
    validate_plugin_manifest(spec, &manifest)?;
    let safe_version = safe_path_segment(&manifest.version)
        .ok_or_else(|| format!("invalid {} plugin version: {}", spec.name, manifest.version))?;
    let final_dir = extension_install_root(&manifest.id).join(safe_version);

    if final_dir.exists() {
        if let Ok(package) = installed_plugin_package_from_root(spec, &final_dir) {
            let _ = fs::remove_dir_all(&extract_dir);
            return Ok(package);
        }
        fs::remove_dir_all(&final_dir).map_err(|err| {
            format!(
                "failed to replace {} plugin package {}: {}",
                spec.name,
                final_dir.to_string_lossy(),
                err
            )
        })?;
    }

    fs::rename(&extracted_root, &final_dir).map_err(|err| {
        format!(
            "failed to install {} plugin package into {}: {}",
            spec.name,
            final_dir.to_string_lossy(),
            err
        )
    })?;
    let _ = fs::remove_dir_all(&extract_dir);

    installed_plugin_package_from_root(spec, &final_dir)
}

fn resolve_installed_plugin_package(
    spec: BuiltinNodeExtensionSpec,
) -> Result<Option<InstalledPluginPackage>, String> {
    let install_root = extension_install_root(spec.id);
    let Ok(entries) = fs::read_dir(install_root) else {
        return Ok(None);
    };

    let mut packages = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || is_hidden_install_dir(&path) {
            continue;
        }
        if let Ok(package) = installed_plugin_package_from_root(spec, &path) {
            packages.push(package);
        }
    }

    Ok(packages
        .into_iter()
        .max_by(|left, right| compare_versions(&left.manifest.version, &right.manifest.version)))
}

fn installed_plugin_package_from_root(
    spec: BuiltinNodeExtensionSpec,
    root_dir: &Path,
) -> Result<InstalledPluginPackage, String> {
    let manifest = read_plugin_manifest(root_dir)?;
    validate_plugin_manifest(spec, &manifest)?;
    let entry_path = plugin_entry_path(root_dir, &manifest)?;
    validate_plugin_entry(entry_path.clone(), spec)?;

    Ok(InstalledPluginPackage {
        manifest,
        root_dir: root_dir.to_path_buf(),
        entry_path,
    })
}

fn read_plugin_manifest(root_dir: &Path) -> Result<PluginManifest, String> {
    let path = root_dir.join("plugin.json");
    let content = fs::read_to_string(&path).map_err(|err| {
        format!(
            "failed to read plugin manifest {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;
    serde_json::from_str::<PluginManifest>(&content).map_err(|err| {
        format!(
            "failed to parse plugin manifest {}: {}",
            path.to_string_lossy(),
            err
        )
    })
}

fn validate_plugin_manifest(
    spec: BuiltinNodeExtensionSpec,
    manifest: &PluginManifest,
) -> Result<(), String> {
    if manifest.id != spec.id {
        return Err(format!(
            "unexpected {} plugin id: {}",
            spec.name, manifest.id
        ));
    }
    if manifest.name.trim().is_empty() {
        return Err(format!("{} plugin manifest name is empty", spec.name));
    }
    if manifest.version.trim().is_empty() {
        return Err(format!("{} plugin manifest version is empty", spec.name));
    }
    if manifest.runtime != "nodejs" {
        return Err(format!(
            "{} plugin runtime must be nodejs; got {}",
            spec.name, manifest.runtime
        ));
    }
    if safe_relative_path(&manifest.entry).is_none() {
        return Err(format!(
            "{} plugin entry is not safe: {}",
            spec.name, manifest.entry
        ));
    }
    Ok(())
}

fn plugin_entry_path(root_dir: &Path, manifest: &PluginManifest) -> Result<PathBuf, String> {
    safe_relative_path(&manifest.entry)
        .map(|entry| root_dir.join(entry))
        .ok_or_else(|| format!("plugin entry is not safe: {}", manifest.entry))
}

fn validate_plugin_entry(path: PathBuf, spec: BuiltinNodeExtensionSpec) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "built-in {} plugin entry not found: {}",
            spec.name,
            path.to_string_lossy()
        ))
    }
}

fn plugin_root_from_entry(entry: &Path) -> Result<PathBuf, String> {
    entry
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "could not resolve plugin root from {}",
                entry.to_string_lossy()
            )
        })
}

fn bundled_plugin_package_path(spec: BuiltinNodeExtensionSpec) -> Result<PathBuf, String> {
    bundled_plugin_package_candidates(spec)
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| format!("built-in {} plugin package was not found", spec.name))
}

fn bundled_plugin_package_candidates(spec: BuiltinNodeExtensionSpec) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("builtin-plugin-packages"));
            candidates.push(exe_dir.join("resources").join("builtin-plugin-packages"));
            if let Some(contents_dir) = exe_dir.parent() {
                candidates.push(
                    contents_dir
                        .join("Resources")
                        .join("builtin-plugin-packages"),
                );
                if let Some(src_tauri_dir) = contents_dir.parent() {
                    candidates.push(src_tauri_dir.join("builtin-plugin-packages"));
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("builtin-plugin-packages"));
        candidates.push(cwd.join("src-tauri").join("builtin-plugin-packages"));
    }

    dedupe_paths(
        candidates
            .into_iter()
            .map(|path| path.join(spec.bundled_package_file))
            .collect(),
    )
}

fn extension_install_root(extension_id: &str) -> PathBuf {
    codexl_home_dir().join("extensions").join(extension_id)
}

fn is_hidden_install_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

fn download_extension_package(
    spec: BuiltinNodeExtensionSpec,
    url: &str,
    version: Option<&str>,
) -> Result<PathBuf, String> {
    let downloads_dir = codexl_home_dir().join("downloads").join("extensions");
    fs::create_dir_all(&downloads_dir).map_err(|err| err.to_string())?;
    let archive_name = extension_archive_name(spec, url, version);
    let archive_path = downloads_dir.join(archive_name);
    let bytes = http_get_bytes(url)?;
    fs::write(&archive_path, bytes).map_err(|err| {
        format!(
            "failed to write {} plugin package {}: {}",
            spec.name,
            archive_path.to_string_lossy(),
            err
        )
    })?;
    Ok(archive_path)
}

fn extension_archive_name(
    spec: BuiltinNodeExtensionSpec,
    url: &str,
    version: Option<&str>,
) -> String {
    let from_url = url
        .split('?')
        .next()
        .and_then(|value| value.rsplit('/').next())
        .filter(|value| value.ends_with(".tar.gz") || value.ends_with(".tgz"));
    if let Some(name) = from_url {
        return name.to_string();
    }
    format!("{}-{}.tar.gz", spec.id, version.unwrap_or("remote"))
}

fn extract_plugin_package_archive(
    spec: BuiltinNodeExtensionSpec,
    archive_path: &Path,
    extract_dir: &Path,
) -> Result<(), String> {
    let archive_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !archive_name.ends_with(".tar.gz") && !archive_name.ends_with(".tgz") {
        return Err(format!(
            "{} plugin packages must be .tar.gz archives",
            spec.name
        ));
    }
    validate_tar_archive_paths(spec, archive_path)?;
    run_command(
        Command::new("tar")
            .arg("-xzf")
            .arg(archive_path)
            .arg("-C")
            .arg(extract_dir),
        &format!("failed to extract {} plugin package", spec.name),
    )
}

fn validate_tar_archive_paths(
    spec: BuiltinNodeExtensionSpec,
    archive_path: &Path,
) -> Result<(), String> {
    let output = Command::new("tar")
        .arg("-tzf")
        .arg(archive_path)
        .output()
        .map_err(|err| format!("failed to inspect {} plugin package: {}", spec.name, err))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("failed to inspect {} plugin package", spec.name)
        } else {
            format!("failed to inspect {} plugin package: {}", spec.name, stderr)
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for entry in stdout.lines() {
        if !is_safe_archive_entry(entry) {
            return Err(format!(
                "unsafe path in {} plugin package: {}",
                spec.name, entry
            ));
        }
    }
    Ok(())
}

fn extracted_plugin_root(extract_dir: &Path) -> Result<PathBuf, String> {
    if extract_dir.join("plugin.json").is_file() {
        return Ok(extract_dir.to_path_buf());
    }
    fs::read_dir(extract_dir)
        .map_err(|err| err.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && path.join("plugin.json").is_file())
        .ok_or_else(|| "Bot plugin package did not contain plugin.json".to_string())
}

fn is_safe_archive_entry(entry: &str) -> bool {
    let trimmed = entry.trim_end_matches('/');
    if trimmed == "." {
        return true;
    }
    if trimmed.is_empty() || trimmed.contains('\\') {
        return false;
    }
    safe_relative_path(trimmed).is_some()
}

fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return None;
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => safe.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

fn safe_path_segment(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed == "."
        || trimmed == ".."
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn validate_remote_release(
    spec: BuiltinNodeExtensionSpec,
    release: &RemotePluginRelease,
) -> Result<(), String> {
    if release.id.as_deref().filter(|id| *id != spec.id).is_some() {
        return Err(format!(
            "unexpected {} plugin update id: {}",
            spec.name,
            release.id.as_deref().unwrap_or_default()
        ));
    }
    if release.version.trim().is_empty() {
        return Err(format!(
            "{} plugin update manifest version is empty",
            spec.name
        ));
    }
    let _ = release.package_url()?;
    Ok(())
}

fn ensure_node_runtime() -> Result<NodeRuntime, String> {
    if let Some(runtime) = resolve_explicit_node_runtime()? {
        return Ok(runtime);
    }
    if let Some(runtime) = resolve_system_node_runtime() {
        return Ok(runtime);
    }
    if let Some(runtime) = resolve_managed_node_runtime() {
        return Ok(runtime);
    }
    download_node_runtime()
}

fn resolve_existing_node_runtime() -> Option<NodeRuntime> {
    resolve_explicit_node_runtime()
        .ok()
        .flatten()
        .or_else(resolve_system_node_runtime)
        .or_else(resolve_managed_node_runtime)
}

fn resolve_explicit_node_runtime() -> Result<Option<NodeRuntime>, String> {
    let Some(path) = env_path(NODE_PATH_ENV) else {
        return Ok(None);
    };
    if !path.is_file() {
        return Err(format!(
            "CODEXL_NODE_PATH does not exist: {}",
            path.to_string_lossy()
        ));
    }
    match valid_node_runtime(&path, RuntimeSource::Explicit) {
        Some(runtime) => Ok(Some(runtime)),
        None => Err(format!(
            "CODEXL_NODE_PATH must point to Node.js {}+; got {}",
            MIN_NODE_MAJOR,
            node_version(&path).unwrap_or_else(|| "unknown".to_string())
        )),
    }
}

fn resolve_system_node_runtime() -> Option<NodeRuntime> {
    system_node_candidates()
        .into_iter()
        .find_map(|path| valid_node_runtime(&path, RuntimeSource::System))
}

fn resolve_managed_node_runtime() -> Option<NodeRuntime> {
    managed_node_candidates()
        .into_iter()
        .find_map(|path| valid_node_runtime(&path, RuntimeSource::Managed))
}

fn valid_node_runtime(path: &Path, source: RuntimeSource) -> Option<NodeRuntime> {
    let version = node_version(path)?;
    if parse_node_major(&version)? < MIN_NODE_MAJOR {
        return None;
    }
    Some(NodeRuntime {
        executable: path.to_path_buf(),
        source,
        version,
    })
}

fn node_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_node_major(version: &str) -> Option<u32> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
}

fn system_node_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = which("node") {
        candidates.push(path);
    }
    if cfg!(windows) {
        if let Some(nvm_symlink) = env_path("NVM_SYMLINK") {
            candidates.push(nvm_symlink.join("node.exe"));
        }
        if let Some(program_files) = env_path("ProgramFiles") {
            candidates.push(program_files.join("nodejs").join("node.exe"));
        }
        if let Some(program_files_x86) = env_path("ProgramFiles(x86)") {
            candidates.push(program_files_x86.join("nodejs").join("node.exe"));
        }
        if let Some(local_app_data) = env_path("LOCALAPPDATA") {
            candidates.push(
                local_app_data
                    .join("Programs")
                    .join("nodejs")
                    .join("node.exe"),
            );
        }
        candidates.push(PathBuf::from(r"C:\Program Files\nodejs\node.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files (x86)\nodejs\node.exe"));
    } else {
        if cfg!(target_os = "macos") {
            candidates.push(PathBuf::from("/opt/homebrew/bin/node"));
            candidates.push(PathBuf::from("/usr/local/bin/node"));
            candidates.push(PathBuf::from("/usr/bin/node"));
        } else {
            candidates.push(PathBuf::from("/usr/local/bin/node"));
            candidates.push(PathBuf::from("/usr/bin/node"));
        }
    }
    dedupe_paths(candidates)
}

fn which(name: &str) -> Result<PathBuf, String> {
    let path_var = std::env::var_os("PATH").ok_or_else(|| "PATH is not set".to_string())?;
    let executable_names = executable_names(name);
    for dir in std::env::split_paths(&path_var) {
        for executable_name in &executable_names {
            let candidate = dir.join(executable_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!("{} not found in PATH", name))
}

fn executable_names(name: &str) -> Vec<OsString> {
    if cfg!(windows) {
        vec![format!("{}.exe", name).into(), name.into()]
    } else {
        vec![name.into()]
    }
}

fn managed_node_candidates() -> Vec<PathBuf> {
    let root = node_runtime_root();
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            candidates.push(node_executable_in_dir(&path));
        }
    }
    dedupe_paths(candidates)
}

fn download_node_runtime() -> Result<NodeRuntime, String> {
    let package = node_package()?;
    let dist_base = std::env::var(NODE_DIST_BASE_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NODE_DIST_BASE.to_string())
        .trim_end_matches('/')
        .to_string();
    let release_base = format!("{}/{}", dist_base, NODE_RELEASE_LINE);
    let shasums_url = format!("{}/SHASUMS256.txt", release_base);
    let shasums = http_get_string(&shasums_url)?;
    let archive_name = select_node_archive(&shasums, &package)?;
    let archive_url = format!("{}/{}", release_base, archive_name);

    let downloads_dir = codexl_home_dir().join("downloads");
    fs::create_dir_all(&downloads_dir).map_err(|err| err.to_string())?;
    let archive_path = downloads_dir.join(&archive_name);
    let bytes = http_get_bytes(&archive_url)?;
    fs::write(&archive_path, bytes).map_err(|err| {
        format!(
            "failed to write Node.js archive {}: {}",
            archive_path.to_string_lossy(),
            err
        )
    })?;

    let extract_dir =
        node_runtime_root().join(format!(".extract-{}-{}", std::process::id(), unix_millis()));
    fs::create_dir_all(&extract_dir).map_err(|err| err.to_string())?;
    extract_archive(&archive_path, &extract_dir, &package)?;

    let extracted_root = first_child_dir(&extract_dir).ok_or_else(|| {
        format!(
            "Node.js archive did not extract a runtime directory: {}",
            archive_path.to_string_lossy()
        )
    })?;
    let final_dir = node_runtime_root().join(
        extracted_root
            .file_name()
            .ok_or_else(|| "Node.js runtime directory has no name".to_string())?,
    );
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|err| {
            format!(
                "failed to replace existing Node.js runtime {}: {}",
                final_dir.to_string_lossy(),
                err
            )
        })?;
    }
    fs::rename(&extracted_root, &final_dir).map_err(|err| {
        format!(
            "failed to install Node.js runtime into {}: {}",
            final_dir.to_string_lossy(),
            err
        )
    })?;
    let _ = fs::remove_dir_all(&extract_dir);

    valid_node_runtime(&node_executable_in_dir(&final_dir), RuntimeSource::Managed)
        .ok_or_else(|| "downloaded Node.js runtime is not executable".to_string())
}

fn node_package() -> Result<NodePackage, String> {
    node_package_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn node_package_for(os: &str, arch: &str) -> Result<NodePackage, String> {
    let platform = match os {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "win",
        other => {
            return Err(format!(
                "automatic Node.js download is not supported on {}; install Node.js {}+ and retry",
                other, MIN_NODE_MAJOR
            ))
        }
    };
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => {
            return Err(format!(
                "automatic Node.js download is not supported on {} {}; install Node.js {}+ and retry",
                platform, other, MIN_NODE_MAJOR
            ))
        }
    };
    let extension = if platform == "win" { "zip" } else { "tar.gz" };
    Ok(NodePackage {
        platform: platform.to_string(),
        arch: arch.to_string(),
        extension: extension.to_string(),
    })
}

fn select_node_archive(shasums: &str, package: &NodePackage) -> Result<String, String> {
    let suffix = format!(
        "-{}-{}.{}",
        package.platform, package.arch, package.extension
    );
    shasums
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .find(|filename| filename.starts_with("node-v") && filename.ends_with(&suffix))
        .map(ToString::to_string)
        .ok_or_else(|| {
            format!(
                "Node.js archive for {} {} was not found in {}",
                package.platform, package.arch, NODE_RELEASE_LINE
            )
        })
}

fn http_get_string(url: &str) -> Result<String, String> {
    http_get(url, |response| async move {
        response.text().await.map_err(|err| err.to_string())
    })
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    http_get(url, |response| async move {
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|err| err.to_string())
    })
}

fn http_get<T, F, Fut>(url: &str, convert: F) -> Result<T, String>
where
    F: FnOnce(reqwest::Response) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    runtime.block_on(async move {
        let response = reqwest::get(url)
            .await
            .map_err(|err| format!("failed to download {}: {}", url, err))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("failed to download {}: HTTP {}", url, status));
        }
        convert(response).await
    })
}

fn extract_archive(
    archive_path: &Path,
    extract_dir: &Path,
    package: &NodePackage,
) -> Result<(), String> {
    if package.extension == "zip" {
        let script = format!(
            "Expand-Archive -LiteralPath {} -DestinationPath {} -Force",
            powershell_quote(&archive_path.to_string_lossy()),
            powershell_quote(&extract_dir.to_string_lossy())
        );
        run_command(
            Command::new(powershell_executable())
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(script),
            "failed to extract Node.js zip archive",
        )
    } else {
        run_command(
            Command::new("tar")
                .arg("-xzf")
                .arg(archive_path)
                .arg("-C")
                .arg(extract_dir),
            "failed to extract Node.js tar archive",
        )
    }
}

fn powershell_executable() -> &'static str {
    if cfg!(windows) {
        "powershell.exe"
    } else {
        "powershell"
    }
}

fn run_command(command: &mut Command, message: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|err| format!("{}: {}", message, err))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(message.to_string())
    } else {
        Err(format!("{}: {}", message, stderr))
    }
}

fn first_child_dir(path: &Path) -> Option<PathBuf> {
    fs::read_dir(path)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
}

fn node_executable_in_dir(dir: &Path) -> PathBuf {
    node_executable_in_dir_for(dir, cfg!(windows))
}

fn node_executable_in_dir_for(dir: &Path, windows: bool) -> PathBuf {
    if windows {
        dir.join("node.exe")
    } else {
        dir.join("bin").join("node")
    }
}

fn node_runtime_root() -> PathBuf {
    codexl_home_dir().join("runtimes").join("node")
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(expand_home_path)
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn expand_home_path(value: String) -> PathBuf {
    let trimmed = value.trim();
    if trimmed == "~" {
        return user_home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = user_home_dir() {
            return home.join(rest);
        }
    }
    if let Some(rest) = trimmed.strip_prefix("~\\") {
        if let Some(home) = user_home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

pub(crate) fn codexl_home_dir() -> PathBuf {
    if let Some(path) = env_path(CODEXL_HOME_ENV) {
        return path;
    }
    if cfg!(windows) {
        if let Some(app_data) = env_path("APPDATA") {
            return app_data.join("CodexL");
        }
    }
    user_home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codexl")
}

fn user_home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path_without_home_expansion("USERPROFILE")
            .or_else(|| {
                let drive = env_string("HOMEDRIVE")?;
                let path = env_string("HOMEPATH")?;
                Some(PathBuf::from(format!("{}{}", drive, path)))
            })
            .or_else(|| env_path_without_home_expansion("HOME"))
    } else {
        env_path_without_home_expansion("HOME")
    }
}

fn env_path_without_home_expansion(name: &str) -> Option<PathBuf> {
    env_string(name).map(PathBuf::from)
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    for path in paths {
        if !deduped.iter().any(|item: &PathBuf| item == &path) {
            deduped.push(path);
        }
    }
    deduped
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == Ordering::Greater
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left_parts = version_parts(left);
    let right_parts = version_parts(right);
    let max_len = left_parts.len().max(right_parts.len());
    for index in 0..max_len {
        let left_value = *left_parts.get(index).unwrap_or(&0);
        let right_value = *right_parts.get(index).unwrap_or(&0);
        match left_value.cmp(&right_value) {
            Ordering::Equal => {}
            order => return order,
        }
    }
    Ordering::Equal
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

impl RuntimeSource {
    fn as_str(self) -> &'static str {
        match self {
            RuntimeSource::Explicit => "explicit",
            RuntimeSource::System => "system",
            RuntimeSource::Managed => "managed",
        }
    }
}

struct NodePackage {
    platform: String,
    arch: String,
    extension: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_node_major_versions() {
        assert_eq!(parse_node_major("v22.11.0"), Some(22));
        assert_eq!(parse_node_major("20.10.0"), Some(20));
        assert_eq!(parse_node_major("not-node"), None);
    }

    #[test]
    fn selects_matching_node_archive() {
        let package = NodePackage {
            platform: "darwin".to_string(),
            arch: "arm64".to_string(),
            extension: "tar.gz".to_string(),
        };
        let shasums = "\
abc  node-v22.9.0-darwin-x64.tar.gz
def  node-v22.9.0-darwin-arm64.tar.gz
";
        assert_eq!(
            select_node_archive(shasums, &package).unwrap(),
            "node-v22.9.0-darwin-arm64.tar.gz"
        );
    }

    #[test]
    fn selects_node_packages_for_macos_and_windows() {
        let mac_arm = node_package_for("macos", "aarch64").unwrap();
        assert_eq!(mac_arm.platform, "darwin");
        assert_eq!(mac_arm.arch, "arm64");
        assert_eq!(mac_arm.extension, "tar.gz");

        let mac_x64 = node_package_for("macos", "x86_64").unwrap();
        assert_eq!(mac_x64.platform, "darwin");
        assert_eq!(mac_x64.arch, "x64");
        assert_eq!(mac_x64.extension, "tar.gz");

        let windows_x64 = node_package_for("windows", "x86_64").unwrap();
        assert_eq!(windows_x64.platform, "win");
        assert_eq!(windows_x64.arch, "x64");
        assert_eq!(windows_x64.extension, "zip");

        let windows_arm = node_package_for("windows", "aarch64").unwrap();
        assert_eq!(windows_arm.platform, "win");
        assert_eq!(windows_arm.arch, "arm64");
        assert_eq!(windows_arm.extension, "zip");
    }

    #[test]
    fn selects_matching_windows_node_archive() {
        let package = node_package_for("windows", "x86_64").unwrap();
        let shasums = "\
abc  node-v22.9.0-win-arm64.zip
def  node-v22.9.0-win-x64.zip
ghi  node-v22.9.0-darwin-x64.tar.gz
";
        assert_eq!(
            select_node_archive(shasums, &package).unwrap(),
            "node-v22.9.0-win-x64.zip"
        );
    }

    #[test]
    fn resolves_node_executable_location_by_platform() {
        let root = Path::new("node-v22.9.0-win-x64");
        assert_eq!(
            node_executable_in_dir_for(root, true),
            root.join("node.exe")
        );
        assert_eq!(
            node_executable_in_dir_for(root, false),
            root.join("bin").join("node")
        );
    }

    #[test]
    fn compares_extension_versions() {
        assert!(version_is_newer("0.2.0", "0.1.9"));
        assert!(version_is_newer("1.0.0", "0.9.9"));
        assert!(!version_is_newer("0.1.0", "0.1.0"));
        assert!(!version_is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn validates_safe_package_paths() {
        assert!(is_safe_archive_entry("plugin.json"));
        assert!(is_safe_archive_entry("./stdio/stdio.js"));
        assert!(is_safe_archive_entry("stdio/"));
        assert!(!is_safe_archive_entry("../plugin.json"));
        assert!(!is_safe_archive_entry("/tmp/plugin.json"));
        assert!(!is_safe_archive_entry("stdio\\stdio.js"));
        assert!(safe_relative_path("stdio/stdio.js").is_some());
        assert!(safe_relative_path("../stdio.js").is_none());
    }
}
