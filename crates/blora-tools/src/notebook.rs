// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Read and edit Jupyter notebooks without executing them.

use blora_exec::LocalBackend;
use blora_types::{BloraError, Result};
use serde_json::{Value, json};

const MAX_NOTEBOOK_BYTES: usize = 5 * 1024 * 1024;

pub fn execute(backend: &LocalBackend, arguments: &Value) -> Result<String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| BloraError::Other("notebook_edit needs path".to_owned()))?;
    let action = arguments
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| BloraError::Other("notebook_edit needs action".to_owned()))?;
    let bytes = backend.read_media_bytes(path)?;
    if bytes.len() > MAX_NOTEBOOK_BYTES {
        return Err(BloraError::Other(format!(
            "notebook exceeds {MAX_NOTEBOOK_BYTES} bytes"
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| BloraError::Other("notebook is not UTF-8".to_owned()))?;
    let mut notebook: Value = serde_json::from_str(&text)
        .map_err(|err| BloraError::Other(format!("invalid notebook JSON: {err}")))?;
    if !notebook.get("cells").is_some_and(Value::is_array) {
        return Err(BloraError::Other(
            "notebook has no cells array (nbformat 4 expected)".to_owned(),
        ));
    }
    if action == "read" {
        return Ok(render(&notebook));
    }
    let index = locate(&notebook, arguments)?;
    let cells = notebook
        .get_mut("cells")
        .and_then(Value::as_array_mut)
        .expect("checked cells");
    match action {
        "insert" => {
            let at = arguments
                .get("index")
                .and_then(Value::as_u64)
                .map_or(cells.len(), |value| value as usize)
                .min(cells.len());
            let cell_type = arguments
                .get("cell_type")
                .and_then(Value::as_str)
                .unwrap_or("markdown");
            if !matches!(cell_type, "code" | "markdown" | "raw") {
                return Err(BloraError::Other(format!("invalid cell_type: {cell_type}")));
            }
            let source = required_source(arguments)?;
            cells.insert(at, make_cell(cell_type, &source));
            write(backend, path, &notebook)?;
            Ok(format!("inserted {cell_type} cell at {at}"))
        }
        "replace" | "edit" => {
            let index = index
                .ok_or_else(|| BloraError::Other(format!("{action} needs cell_id or index")))?;
            let cell = cells
                .get_mut(index)
                .ok_or_else(|| BloraError::Other(format!("cell index out of range: {index}")))?;
            let source = if action == "replace" {
                required_source(arguments)?
            } else {
                let old = arguments
                    .get("old_string")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        BloraError::Other("notebook edit needs old_string".to_owned())
                    })?;
                let new = arguments
                    .get("new_string")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        BloraError::Other("notebook edit needs new_string".to_owned())
                    })?;
                let current = source_text(cell);
                let count = current.matches(old).count();
                if count != 1 {
                    return Err(BloraError::Other(format!(
                        "old_string must match exactly once (found {count})"
                    )));
                }
                current.replacen(old, new, 1)
            };
            set_source(cell, &source);
            clear_outputs(cell);
            write(backend, path, &notebook)?;
            Ok(format!("updated cell {}", index))
        }
        "delete" => {
            let index = index
                .ok_or_else(|| BloraError::Other("delete needs cell_id or index".to_owned()))?;
            if index >= cells.len() {
                return Err(BloraError::Other(format!(
                    "cell index out of range: {index}"
                )));
            }
            cells.remove(index);
            write(backend, path, &notebook)?;
            Ok(format!("deleted cell {index}"))
        }
        "move" => {
            let from =
                index.ok_or_else(|| BloraError::Other("move needs cell_id or index".to_owned()))?;
            if from >= cells.len() {
                return Err(BloraError::Other(format!(
                    "cell index out of range: {from}"
                )));
            }
            let to = arguments
                .get("to")
                .and_then(Value::as_u64)
                .map_or(cells.len().saturating_sub(1), |value| value as usize)
                .min(cells.len().saturating_sub(1));
            let cell = cells.remove(from);
            cells.insert(to, cell);
            write(backend, path, &notebook)?;
            Ok(format!("moved cell {from} to {to}"))
        }
        other => Err(BloraError::Other(format!(
            "unknown notebook action: {other}"
        ))),
    }
}

fn locate(notebook: &Value, arguments: &Value) -> Result<Option<usize>> {
    if let Some(index) = arguments.get("index").and_then(Value::as_u64) {
        return Ok(Some(index as usize));
    }
    let Some(cell_id) = arguments.get("cell_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let cells = notebook
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| BloraError::Other("notebook has no cells".to_owned()))?;
    Ok(cells
        .iter()
        .position(|cell| cell.get("id").and_then(Value::as_str) == Some(cell_id)))
}

fn required_source(arguments: &Value) -> Result<String> {
    arguments
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| BloraError::Other("notebook action needs source".to_owned()))
}

fn make_cell(cell_type: &str, source: &str) -> Value {
    let mut cell = json!({
        "cell_type": cell_type,
        "id": format!("cell_{}", uuid::Uuid::now_v7().simple()),
        "metadata": {},
        "source": source,
    });
    if cell_type == "code" {
        cell["execution_count"] = Value::Null;
        cell["outputs"] = json!([]);
    }
    cell
}

fn source_text(cell: &Value) -> String {
    match cell.get("source") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(lines)) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn set_source(cell: &mut Value, source: &str) {
    cell["source"] = Value::String(source.to_owned());
}

fn clear_outputs(cell: &mut Value) {
    if cell.get("cell_type").and_then(Value::as_str) == Some("code") {
        cell["outputs"] = json!([]);
        cell["execution_count"] = Value::Null;
    }
}

fn write(backend: &LocalBackend, path: &str, notebook: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(notebook)
        .map_err(|err| BloraError::Other(format!("serialize notebook: {err}")))?;
    backend.write_file(path, &format!("{text}\n"))
}

fn render(notebook: &Value) -> String {
    let cells = notebook
        .get("cells")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if cells.is_empty() {
        return "(empty notebook)".to_owned();
    }
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let kind = cell
                .get("cell_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let id = cell.get("id").and_then(Value::as_str).unwrap_or("");
            let source = source_text(cell);
            let source = if source.chars().count() > 4_000 {
                format!("{}…", source.chars().take(4_000).collect::<String>())
            } else {
                source
            };
            format!("[{index}] {kind} {id}\n{source}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_policy::Policy;

    #[test]
    fn reads_edits_and_moves_cells() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unit.ipynb");
        std::fs::write(
            &path,
            r##"{"cells":[{"cell_type":"markdown","id":"a","metadata":{},"source":"# Title"},{"cell_type":"code","id":"b","metadata":{},"source":"print(1)","outputs":[],"execution_count":1}],"metadata":{},"nbformat":4,"nbformat_minor":5}"##,
        )
        .unwrap();
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        let read = execute(&backend, &json!({"path":"unit.ipynb","action":"read"})).unwrap();
        assert!(read.contains("[0] markdown a"));
        execute(
            &backend,
            &json!({"path":"unit.ipynb","action":"edit","cell_id":"b","old_string":"print(1)","new_string":"print(2)"}),
        )
        .unwrap();
        execute(
            &backend,
            &json!({"path":"unit.ipynb","action":"insert","index":0,"cell_type":"markdown","source":"intro"}),
        )
        .unwrap();
        execute(
            &backend,
            &json!({"path":"unit.ipynb","action":"move","index":0,"to":2}),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let notebook: Value = serde_json::from_str(&text).unwrap();
        let cells = notebook["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0]["id"], "a");
        assert_eq!(source_text(&cells[1]), "print(2)");
        assert_eq!(cells[1]["execution_count"], Value::Null);
    }
}
