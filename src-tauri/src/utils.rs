use anyhow::{anyhow, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use std::borrow::Cow;

pub fn normalize_publication_url(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Publication URL cannot be empty."));
    }

    let candidate = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.contains('.') {
        format!("https://{trimmed}")
    } else {
        format!("https://{trimmed}.substack.com")
    };

    let parsed = url::Url::parse(&candidate).map_err(|_| anyhow!("Invalid publication URL."))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("Publication URL must include a valid host."))?;
    let mut base = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        base.push(':');
        base.push_str(&port.to_string());
    }
    Ok(base)
}

pub fn parse_datetime_flexible(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .or_else(|_| DateTime::parse_from_rfc2822(value).map(|v| v.with_timezone(&Utc)))
        .ok()
}

pub fn sanitize_filename(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ' ' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    let mut clean = result.trim().replace("  ", " ");
    while clean.contains("  ") {
        clean = clean.replace("  ", " ");
    }
    clean = clean.trim_matches('.').to_string();
    if clean.is_empty() {
        "untitled".to_string()
    } else {
        clean.chars().take(120).collect()
    }
}

pub fn decode_data_url(data_url: &str) -> Result<(Vec<u8>, String)> {
    let (meta, body) = data_url
        .split_once(',')
        .ok_or_else(|| anyhow!("Invalid data URL format."))?;
    if !meta.ends_with(";base64") {
        return Err(anyhow!("Only base64 data URLs are supported."));
    }
    let mime_type = meta
        .strip_prefix("data:")
        .unwrap_or(meta)
        .strip_suffix(";base64")
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|_| anyhow!("Failed to decode base64 cover image."))?;
    Ok((bytes, mime_type))
}

pub fn media_type_to_extension(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "img",
    }
}

pub fn escape_xml(value: &str) -> Cow<'_, str> {
    if !(value.contains('&') || value.contains('<') || value.contains('>') || value.contains('"') || value.contains('\'')) {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len() + 8);
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    Cow::Owned(out)
}
