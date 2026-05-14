use super::*;

#[derive(Debug, Clone)]
struct WebFilePickerEntry {
    hidden: bool,
    name: String,
    path: String,
}

pub(super) fn is_web_file_picker_message(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(WEB_FILE_PICKER_LIST_MESSAGE)
}

pub(super) fn dispatch_web_file_picker_message(message: Value) -> Result<Value, String> {
    match message.get("type").and_then(Value::as_str) {
        Some(WEB_FILE_PICKER_LIST_MESSAGE) => {
            let path = message.get("path").and_then(Value::as_str);
            Ok(json!({
                "messages": [],
                "value": web_file_picker_directory_payload(path)?,
            }))
        }
        Some(message_type) => Err(format!(
            "unsupported web file picker request: {}",
            message_type
        )),
        None => Err("missing web file picker request type".to_string()),
    }
}

pub(super) fn web_file_picker_directory_payload(path: Option<&str>) -> Result<Value, String> {
    let directory = normalize_web_file_picker_path(path)?;
    let metadata = fs::metadata(&directory)
        .map_err(|err| format!("cannot open {}: {}", directory.display(), err))?;
    if !metadata.is_dir() {
        return Err(format!("not a directory: {}", directory.display()));
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&directory)
        .map_err(|err| format!("cannot read {}: {}", directory.display(), err))?;
    for entry in read_dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !web_file_picker_entry_is_directory(&entry) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() {
            continue;
        }
        entries.push(WebFilePickerEntry {
            hidden: name.starts_with('.'),
            name,
            path: entry.path().to_string_lossy().into_owned(),
        });
    }

    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = entries.len() > WEB_FILE_PICKER_ENTRY_LIMIT;
    entries.truncate(WEB_FILE_PICKER_ENTRY_LIMIT);

    let entries = entries
        .into_iter()
        .map(|entry| {
            json!({
                "hidden": entry.hidden,
                "kind": "directory",
                "name": entry.name,
                "path": entry.path,
            })
        })
        .collect::<Vec<_>>();
    let parent = directory.parent().map(path_to_string);

    Ok(json!({
        "entries": entries,
        "parent": parent,
        "path": path_to_string(&directory),
        "truncated": truncated,
    }))
}

fn web_file_picker_entry_is_directory(entry: &fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_dir() => true,
        Ok(file_type) if file_type.is_symlink() => entry
            .metadata()
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false),
        _ => false,
    }
}

fn normalize_web_file_picker_path(path: Option<&str>) -> Result<PathBuf, String> {
    let trimmed = path.unwrap_or("").trim();
    let mut directory = if trimmed.is_empty() {
        default_web_file_picker_path()
    } else if trimmed == "~" {
        home_directory().unwrap_or_else(default_web_file_picker_path)
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        match home_directory() {
            Some(home) => home.join(rest),
            None => PathBuf::from(trimmed),
        }
    } else {
        PathBuf::from(trimmed)
    };

    if let Ok(canonical) = fs::canonicalize(&directory) {
        directory = canonical;
    }
    Ok(directory)
}

fn default_web_file_picker_path() -> PathBuf {
    home_directory().unwrap_or_else(|| {
        if cfg!(windows) {
            PathBuf::from(r"C:\")
        } else {
            PathBuf::from("/")
        }
    })
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
