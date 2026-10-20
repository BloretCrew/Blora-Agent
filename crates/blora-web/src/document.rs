// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Text extraction for PDF and Office Open XML files.
//!
//! Office files are zip archives of XML parts; they are read with a small
//! central-directory parser plus `flate2`. PDF text comes from the system
//! `pdftotext` (poppler) because a faithful PDF text layer parser is large.

use std::io::Read;
use std::path::Path;

use blora_types::{BloraError, Result};

const MAX_PART_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
}

impl DocumentKind {
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            "xlsx" => Some(Self::Xlsx),
            "pptx" => Some(Self::Pptx),
            _ => None,
        }
    }
}

/// Extract readable text from a document file.
pub fn read_document(path: &Path) -> Result<String> {
    let kind = DocumentKind::from_path(path).ok_or_else(|| {
        BloraError::Other(
            "read_document supports .pdf, .docx, .xlsx, and .pptx; use read_file for text files"
                .to_owned(),
        )
    })?;
    match kind {
        DocumentKind::Pdf => read_pdf(path),
        DocumentKind::Docx => {
            let zip = Zip::open(path)?;
            Ok(docx_text(&zip.read("word/document.xml")?))
        }
        DocumentKind::Pptx => {
            let zip = Zip::open(path)?;
            let mut slides: Vec<(u32, String)> = zip
                .names()
                .filter_map(|name| {
                    let number = name
                        .strip_prefix("ppt/slides/slide")?
                        .strip_suffix(".xml")?
                        .parse()
                        .ok()?;
                    Some((number, name.to_owned()))
                })
                .collect();
            slides.sort();
            let mut out = String::new();
            for (number, name) in slides {
                let text = paragraphs(&zip.read(&name)?, "a:p", "a:t");
                out.push_str(&format!("## Slide {number}\n{text}\n\n"));
            }
            Ok(out.trim_end().to_owned())
        }
        DocumentKind::Xlsx => {
            let zip = Zip::open(path)?;
            xlsx_text(&zip)
        }
    }
}

fn read_pdf(path: &Path) -> Result<String> {
    let output = std::process::Command::new("pdftotext")
        .args(["-layout", "-enc", "UTF-8"])
        .arg(path)
        .arg("-")
        .output()
        .map_err(|err| {
            BloraError::Other(format!(
                "PDF reading needs pdftotext (poppler-utils) on PATH: {err}"
            ))
        })?;
    if !output.status.success() {
        return Err(BloraError::Other(format!(
            "pdftotext failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        return Ok("[the PDF has no text layer; it may be a scanned image]".to_owned());
    }
    Ok(text)
}

fn docx_text(xml: &str) -> String {
    paragraphs(xml, "w:p", "w:t")
}

/// Join the text runs (`run_tag`) of each paragraph (`para_tag`) into lines.
fn paragraphs(xml: &str, para_tag: &str, run_tag: &str) -> String {
    let mut out = Vec::new();
    let close = format!("</{para_tag}>");
    for chunk in xml.split(close.as_str()) {
        let line = runs(chunk, run_tag).join("");
        let line = line.trim();
        if !line.is_empty() {
            out.push(line.to_owned());
        }
    }
    out.join("\n")
}

/// Text inside every `<tag ...>text</tag>` in `xml`.
fn runs(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // Reject longer tag names that share the prefix (w:t vs w:tab).
        if !after.starts_with(['>', ' ', '/']) {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(crate::html::decode_entities(&body[..end]));
        rest = &body[end + close.len()..];
    }
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn xlsx_text(zip: &Zip) -> Result<String> {
    let shared: Vec<String> = match zip.read("xl/sharedStrings.xml") {
        Ok(xml) => xml
            .split("</si>")
            .map(|item| runs(item, "t").join(""))
            .collect(),
        Err(_) => Vec::new(),
    };
    let names = sheet_names(zip);
    let mut sheets: Vec<(u32, String)> = zip
        .names()
        .filter_map(|name| {
            let number = name
                .strip_prefix("xl/worksheets/sheet")?
                .strip_suffix(".xml")?
                .parse()
                .ok()?;
            Some((number, name.to_owned()))
        })
        .collect();
    sheets.sort();
    let mut out = String::new();
    for (number, part) in sheets {
        let label = names
            .get(number as usize - 1)
            .cloned()
            .unwrap_or_else(|| format!("Sheet{number}"));
        out.push_str(&format!("## {label}\n"));
        let xml = zip.read(&part)?;
        for row in xml.split("</row>") {
            let mut cells = Vec::new();
            let mut rest = row;
            while let Some(start) = rest.find("<c ") {
                let after = &rest[start..];
                let Some(gt) = after.find('>') else { break };
                let tag = &after[..gt];
                let (body, next) = if tag.ends_with('/') {
                    ("", &after[gt + 1..])
                } else {
                    let body = &after[gt + 1..];
                    let end = body.find("</c>").unwrap_or(body.len());
                    (&body[..end], &body[(end + 4).min(body.len())..])
                };
                let value = runs(body, "v").into_iter().next().unwrap_or_default();
                let text = match attr(tag, "t") {
                    Some("s") => value
                        .parse::<usize>()
                        .ok()
                        .and_then(|index| shared.get(index).cloned())
                        .unwrap_or_default(),
                    Some("inlineStr") => runs(body, "t").join(""),
                    _ => value,
                };
                cells.push(text.replace(['\t', '\n'], " "));
                rest = next;
            }
            if cells.iter().any(|cell| !cell.is_empty()) {
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
        out.push('\n');
    }
    Ok(out.trim_end().to_owned())
}

fn sheet_names(zip: &Zip) -> Vec<String> {
    let Ok(xml) = zip.read("xl/workbook.xml") else {
        return Vec::new();
    };
    xml.split("<sheet ")
        .skip(1)
        .filter_map(|tag| attr(&format!(" {tag}"), "name").map(crate::html::decode_entities))
        .collect()
}

/// Minimal read-only zip reader (stored and deflated entries).
struct Zip {
    data: Vec<u8>,
    entries: Vec<ZipEntry>,
}

struct ZipEntry {
    name: String,
    method: u16,
    compressed: usize,
    size: usize,
    local_offset: usize,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize)
}

impl Zip {
    fn open(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(BloraError::exec)?;
        Self::parse(data)
    }

    fn parse(data: Vec<u8>) -> Result<Self> {
        let bad = || BloraError::Other("not a valid Office document (zip)".to_owned());
        let search_from = data.len().saturating_sub(65_557);
        let eocd = (search_from..data.len().saturating_sub(21))
            .rev()
            .find(|&at| data[at..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
            .ok_or_else(bad)?;
        let count = u16_at(&data, eocd + 10).ok_or_else(bad)? as usize;
        let mut at = u32_at(&data, eocd + 16).ok_or_else(bad)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if !data
                .get(at..)
                .is_some_and(|rest| rest.starts_with(&[0x50, 0x4b, 0x01, 0x02]))
            {
                return Err(bad());
            }
            let method = u16_at(&data, at + 10).ok_or_else(bad)?;
            let compressed = u32_at(&data, at + 20).ok_or_else(bad)?;
            let size = u32_at(&data, at + 24).ok_or_else(bad)?;
            let name_len = u16_at(&data, at + 28).ok_or_else(bad)? as usize;
            let extra_len = u16_at(&data, at + 30).ok_or_else(bad)? as usize;
            let comment_len = u16_at(&data, at + 32).ok_or_else(bad)? as usize;
            let local_offset = u32_at(&data, at + 42).ok_or_else(bad)?;
            let name = data.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
            entries.push(ZipEntry {
                name: String::from_utf8_lossy(name).into_owned(),
                method,
                compressed,
                size,
                local_offset,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { data, entries })
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }

    fn read(&self, name: &str) -> Result<String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| BloraError::Other(format!("document part {name} is missing")))?;
        let bad = || BloraError::Other(format!("document part {name} is corrupt"));
        if entry.size > MAX_PART_BYTES {
            return Err(BloraError::Other(format!(
                "document part {name} is too large"
            )));
        }
        let at = entry.local_offset;
        let name_len = u16_at(&self.data, at + 26).ok_or_else(bad)? as usize;
        let extra_len = u16_at(&self.data, at + 28).ok_or_else(bad)? as usize;
        let start = at + 30 + name_len + extra_len;
        let raw = self
            .data
            .get(start..start + entry.compressed)
            .ok_or_else(bad)?;
        let bytes = match entry.method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(entry.size);
                flate2::read::DeflateDecoder::new(raw)
                    .take(MAX_PART_BYTES as u64)
                    .read_to_end(&mut out)
                    .map_err(|_| bad())?;
                out
            }
            other => {
                return Err(BloraError::Other(format!(
                    "unsupported zip compression method {other}"
                )));
            }
        };
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a zip with deflated entries for tests.
    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in entries {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(body.as_bytes()).unwrap();
            let compressed = encoder.finish().unwrap();
            let offset = out.len() as u32;
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 8, 0]);
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&compressed);
            central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 8, 0]);
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn write(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn reads_docx_paragraphs() {
        let dir = tempfile::tempdir().unwrap();
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve"> world &amp; co</w:t></w:r></w:p><w:p><w:r><w:t>第二段</w:t></w:r></w:p></w:body></w:document>"#;
        let path = write(dir.path(), "a.docx", &zip(&[("word/document.xml", xml)]));
        assert_eq!(read_document(&path).unwrap(), "Hello world & co\n第二段");
    }

    #[test]
    fn reads_xlsx_with_shared_strings() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "b.xlsx",
            &zip(&[
                (
                    "xl/workbook.xml",
                    r#"<workbook><sheets><sheet name="Budget" sheetId="1"/></sheets></workbook>"#,
                ),
                (
                    "xl/sharedStrings.xml",
                    "<sst><si><t>Item</t></si><si><t>Cost</t></si></sst>",
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Pens</t></is></c><c r="B2"><v>3.5</v></c></row></sheetData></worksheet>"#,
                ),
            ]),
        );
        assert_eq!(
            read_document(&path).unwrap(),
            "## Budget\nItem\tCost\nPens\t3.5"
        );
    }

    #[test]
    fn reads_pptx_slides_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let slide = |text: &str| format!("<p:sld><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>");
        let (two, ten) = (slide("Second"), slide("Tenth"));
        let path = write(
            dir.path(),
            "c.pptx",
            &zip(&[
                ("ppt/slides/slide10.xml", &ten),
                ("ppt/slides/slide2.xml", &two),
            ]),
        );
        assert_eq!(
            read_document(&path).unwrap(),
            "## Slide 2\nSecond\n\n## Slide 10\nTenth"
        );
    }

    #[test]
    fn rejects_unknown_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_document(&write(dir.path(), "x.txt", b"hi")).is_err());
        let err = read_document(&write(dir.path(), "y.docx", b"not a zip")).unwrap_err();
        assert!(err.to_string().contains("not a valid Office document"));
    }

    #[test]
    fn reads_pdf_when_pdftotext_exists() {
        if std::process::Command::new("pdftotext")
            .arg("-v")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let pdf = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 100]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n4 0 obj<</Length 44>>stream\nBT /F1 18 Tf 20 40 Td (Blora PDF test) Tj ET\nendstream endobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
        let path = write(dir.path(), "d.pdf", pdf.as_bytes());
        let text = read_document(&path).unwrap();
        assert!(text.contains("Blora PDF test"), "{text}");
    }
}
