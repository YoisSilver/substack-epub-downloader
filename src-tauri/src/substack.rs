use crate::models::{PostContent, PostSummary, PublicationInfo, PublicationRequest, PublicationResponse};
use crate::utils::{normalize_publication_url, parse_datetime_flexible};
use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use regex::Regex;
use reqwest::Client;
use rss::Channel;
use scraper::{Html, Selector};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const USER_AGENT: &str = "substack-downloader/0.1 (+desktop)";

#[derive(Debug, Clone)]
struct FootnoteEntry {
    id: String,
    number: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct FootnoteCandidate {
    ids: HashSet<String>,
    href_targets: HashSet<String>,
    text: String,
}

#[derive(Debug, Clone)]
struct ProcessedBody {
    plain_text: String,
    epub_body: String,
}

pub fn build_http_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client: {e}"))
}

pub async fn load_publication_posts(request: PublicationRequest) -> Result<PublicationResponse> {
    let base_url = normalize_publication_url(&request.url)?;
    let client = build_http_client()?;

    if let Ok(mut feed_response) = load_from_feed(&client, &base_url).await {
        hydrate_publication_identity(&client, &mut feed_response.publication).await;
        return Ok(feed_response);
    }

    let mut archive_response = load_from_archive(&client, &base_url).await?;
    hydrate_publication_identity(&client, &mut archive_response.publication).await;
    Ok(archive_response)
}

async fn hydrate_publication_identity(client: &Client, publication: &mut PublicationInfo) {
    let needs_author = publication.author.as_ref().map(|value| value.trim().is_empty()).unwrap_or(true);
    let needs_cover = publication
        .author_cover_url
        .as_ref()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true);
    if !needs_author && !needs_cover {
        return;
    }

    let Ok(html) = fetch_text_with_retries(client, &publication.url, 1).await else {
        return;
    };
    let document = Html::parse_document(&html);
    if needs_author {
        publication.author = extract_author(&document, &html);
    }
    if needs_cover {
        publication.author_cover_url = extract_meta_property(&document, "og:image");
    }
}

pub async fn fetch_post_content(client: &Client, summary: &PostSummary, retries: usize) -> Result<PostContent> {
    let html = fetch_text_with_retries(client, &summary.url, retries).await?;
    let document = Html::parse_document(&html);

    let title = extract_meta_property(&document, "og:title")
        .or_else(|| extract_text(&document, "h1"))
        .unwrap_or_else(|| summary.title.clone());
    let author = extract_author(&document, &html).or_else(|| summary.author.clone());
    let published_at = extract_meta_property(&document, "article:published_time").unwrap_or_else(|| summary.published_at.clone());
    let subtitle = extract_meta_property(&document, "og:description").or_else(|| summary.subtitle.clone());
    let cover = extract_meta_property(&document, "og:image").or_else(|| summary.cover_image_url.clone());
    let tags = extract_meta_values(&document, "article:tag");
    let reading_time = parse_reading_time(&html);

    let body_html = extract_body_html(&document).unwrap_or_else(|| {
        extract_text(&document, "main")
            .map(|text| format!("<p>{}</p>", text))
            .unwrap_or_else(|| "<p>No content extracted.</p>".to_string())
    });

    let processed_body = process_body_for_exports(&body_html);

    let normalized = PostSummary {
        id: summary.id.clone(),
        title,
        published_at,
        url: summary.url.clone(),
        author,
        cover_image_url: cover,
        tags: if tags.is_empty() { summary.tags.clone() } else { Some(tags) },
        subtitle,
        summary: summary.summary.clone(),
    };

    Ok(PostContent {
        summary: normalized,
        plain_text: processed_body.plain_text,
        epub_body: processed_body.epub_body,
        reading_time_minutes: reading_time,
        summary_text: summary.summary.clone(),
    })
}

async fn load_from_feed(client: &Client, base_url: &str) -> Result<PublicationResponse> {
    let mut candidates = vec![format!("{base_url}/feed"), format!("{base_url}/rss")];
    if base_url.contains("substack.com") {
        candidates.push(format!("{base_url}/feed?source=desktop"));
    }

    let mut last_error: Option<anyhow::Error> = None;
    for feed_url in candidates {
        match fetch_text_with_retries(client, &feed_url, 2).await {
            Ok(raw_feed) => match Channel::read_from(raw_feed.as_bytes()) {
                Ok(channel) => {
                    let publication = map_publication_from_channel(base_url, &channel);
                    let posts = map_posts_from_channel(&channel);
                    if posts.is_empty() {
                        return Err(anyhow!("Feed loaded but no posts were found."));
                    }
                    return Ok(PublicationResponse { publication, posts });
                }
                Err(error) => {
                    last_error = Some(anyhow!("Failed to parse feed {feed_url}: {error}"));
                }
            },
            Err(error) => {
                last_error = Some(error.context(format!("Failed feed candidate {feed_url}")));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Unable to load publication feed.")))
}

async fn load_from_archive(client: &Client, base_url: &str) -> Result<PublicationResponse> {
    let archive_url = format!("{base_url}/archive");
    let html = fetch_text_with_retries(client, &archive_url, 2).await?;
    let document = Html::parse_document(&html);

    let title = extract_text(&document, "title").unwrap_or_else(|| "Substack publication".to_string());
    let author = extract_author(&document, &html);
    let author_cover_url = extract_meta_property(&document, "og:image");

    let link_selector = Selector::parse("a[href*='/p/']").unwrap();
    let mut seen = HashSet::new();
    let mut posts = Vec::new();
    for (idx, anchor) in document.select(&link_selector).enumerate() {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let full_url = if href.starts_with("http://") || href.starts_with("https://") {
            href.to_string()
        } else {
            format!("{base_url}{}", if href.starts_with('/') { "" } else { "/" }) + href
        };
        if !seen.insert(full_url.clone()) {
            continue;
        }

        let link_text = anchor.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if link_text.is_empty() {
            continue;
        }
        let pseudo_date = (Utc::now() - Duration::seconds(idx as i64)).to_rfc3339();
        posts.push(PostSummary {
            id: full_url.clone(),
            title: link_text,
            published_at: pseudo_date,
            url: full_url,
            author: author.clone(),
            cover_image_url: None,
            tags: None,
            subtitle: None,
            summary: None,
        });
    }

    if posts.is_empty() {
        return Err(anyhow!("Could not discover any posts from feed or archive."));
    }

    Ok(PublicationResponse {
        publication: PublicationInfo {
            url: base_url.to_string(),
            title,
            author,
            author_cover_url,
        },
        posts,
    })
}

fn map_publication_from_channel(base_url: &str, channel: &Channel) -> PublicationInfo {
    let author = channel
        .items()
        .iter()
        .find_map(|item| item.author().map(|a| a.to_string()));
    let title = channel.title().to_string();
    let author_cover_url = channel.image().map(|img| img.url().to_string());

    PublicationInfo {
        url: base_url.to_string(),
        title,
        author,
        author_cover_url,
    }
}

fn map_posts_from_channel(channel: &Channel) -> Vec<PostSummary> {
    let mut posts = channel
        .items()
        .iter()
        .filter_map(|item| {
            let url = item.link()?.to_string();
            let title = item.title().unwrap_or("Untitled post").to_string();
            let pub_date = item
                .pub_date()
                .and_then(parse_datetime_flexible)
                .unwrap_or_else(Utc::now)
                .to_rfc3339();
            let id = item
                .guid()
                .map(|guid| guid.value().to_string())
                .unwrap_or_else(|| url.clone());
            let cover = item.enclosure().map(|enc| enc.url().to_string());
            let subtitle = item.description().map(|desc| desc.to_string());
            let author = item.author().map(|a| a.to_string());

            Some(PostSummary {
                id,
                title,
                published_at: pub_date,
                url,
                author,
                cover_image_url: cover,
                tags: None,
                subtitle,
                summary: None,
            })
        })
        .collect::<Vec<_>>();

    posts.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    posts
}

pub async fn fetch_text_with_retries(client: &Client, url: &str, retries: usize) -> Result<String> {
    let mut delay_ms = 350;
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 0..=retries {
        match client.get(url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(success) => return success.text().await.map_err(|e| anyhow!("Failed reading response body: {e}")),
                Err(error) => last_error = Some(anyhow!("Request failed with status on attempt {}: {}", attempt + 1, error)),
            },
            Err(error) => last_error = Some(anyhow!("Network request failed on attempt {}: {}", attempt + 1, error)),
        }
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        delay_ms *= 2;
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Failed to fetch {url} after retries.")))
}

pub async fn fetch_bytes_with_retries(client: &Client, url: &str, retries: usize) -> Result<Vec<u8>> {
    let mut delay_ms = 350;
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 0..=retries {
        match client.get(url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(success) => {
                    return success
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .map_err(|e| anyhow!("Failed reading binary response body: {e}"))
                }
                Err(error) => last_error = Some(anyhow!("Request failed with status on attempt {}: {}", attempt + 1, error)),
            },
            Err(error) => last_error = Some(anyhow!("Network request failed on attempt {}: {}", attempt + 1, error)),
        }
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        delay_ms *= 2;
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Failed to fetch binary content from {url} after retries.")))
}

fn extract_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let text = document
        .select(&selector)
        .next()?
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_meta_name(document: &Html, name: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[name='{name}']")).ok()?;
    let content = document
        .select(&selector)
        .next()
        .and_then(|node| node.value().attr("content"))?
        .trim()
        .to_string();
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

fn extract_meta_property(document: &Html, property: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[property='{property}']")).ok()?;
    let content = document
        .select(&selector)
        .next()
        .and_then(|node| node.value().attr("content"))?
        .trim()
        .to_string();
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

fn extract_meta_values(document: &Html, property: &str) -> Vec<String> {
    let selector = match Selector::parse(&format!("meta[property='{property}']")) {
        Ok(selector) => selector,
        Err(_) => return Vec::new(),
    };
    document
        .select(&selector)
        .filter_map(|node| node.value().attr("content"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn extract_author(document: &Html, page_html: &str) -> Option<String> {
    let mut candidates = Vec::new();

    if let Some(value) = extract_meta_name(document, "author") {
        candidates.push(value);
    }
    if let Some(value) = extract_meta_name(document, "parsely-author") {
        candidates.push(value);
    }
    if let Some(value) = extract_meta_property(document, "article:author") {
        candidates.push(value);
    }
    if let Some(value) = extract_meta_property(document, "og:article:author") {
        candidates.push(value);
    }
    if let Some(value) = extract_text(document, "[itemprop='author']") {
        candidates.push(value);
    }
    if let Some(value) = extract_text(document, "a[rel='author']") {
        candidates.push(value);
    }
    if let Some(value) = extract_text(document, ".pencraft .byline-name") {
        candidates.push(value);
    }
    if let Some(value) = extract_text(document, ".post-meta .author") {
        candidates.push(value);
    }
    if let Some(value) = extract_author_from_json_ld(page_html) {
        candidates.push(value);
    }

    for candidate in candidates {
        let cleaned = normalize_whitespace(&candidate);
        if cleaned.is_empty() {
            continue;
        }
        let lower = cleaned.to_ascii_lowercase();
        if lower == "substack" || lower == "unknown" {
            continue;
        }
        return Some(cleaned);
    }
    None
}

fn extract_author_from_json_ld(page_html: &str) -> Option<String> {
    let script_regex = Regex::new(
        r#"(?is)<script[^>]*type=["']application/ld\+json["'][^>]*>\s*(?P<body>.*?)\s*</script>"#,
    )
    .expect("valid json-ld regex");
    for captures in script_regex.captures_iter(page_html) {
        let Some(body_match) = captures.name("body") else {
            continue;
        };
        let body = body_match.as_str().trim();
        if body.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(body) else {
            continue;
        };
        let mut found = Vec::new();
        collect_author_names_from_json(&parsed, &mut found);
        for author in found {
            let cleaned = normalize_whitespace(&author);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn collect_author_names_from_json(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(author) = map.get("author") {
                match author {
                    Value::String(name) => output.push(name.to_string()),
                    Value::Object(author_map) => {
                        if let Some(Value::String(name)) = author_map.get("name") {
                            output.push(name.to_string());
                        }
                    }
                    Value::Array(items) => {
                        for item in items {
                            if let Value::Object(author_map) = item {
                                if let Some(Value::String(name)) = author_map.get("name") {
                                    output.push(name.to_string());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            for child in map.values() {
                collect_author_names_from_json(child, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_author_names_from_json(item, output);
            }
        }
        _ => {}
    }
}

fn extract_body_html(document: &Html) -> Option<String> {
    let selectors = [
        ".available-content",
        "article .body",
        "article .markup",
        ".body.markup",
        "article",
        "main",
    ];

    for candidate in selectors {
        if let Ok(selector) = Selector::parse(candidate) {
            if let Some(node) = document.select(&selector).next() {
                let html = node.inner_html();
                if !html.trim().is_empty() {
                    return Some(html);
                }
            }
        }
    }
    None
}

fn parse_reading_time(html: &str) -> Option<u32> {
    let regex = Regex::new(r"(?i)(\d+)\s*min\s*read").ok()?;
    let caps = regex.captures(html)?;
    caps.get(1)?.as_str().parse::<u32>().ok()
}

fn process_body_for_exports(body_html: &str) -> ProcessedBody {
    let footnotes = extract_footnotes(body_html);
    let main_html = remove_footnote_containers(body_html);
    let html_with_markers = replace_footnote_refs_with_tokens(&main_html, &footnotes);

    let plain_text = render_plain_text(&html_with_markers, &footnotes);
    let epub_body = build_epub_body(&html_with_markers, &footnotes);

    ProcessedBody {
        plain_text,
        epub_body,
    }
}

fn extract_footnotes(body_html: &str) -> Vec<FootnoteEntry> {
    let target_ids = collect_footnote_target_ids(body_html);
    let mut seen_target_ids = HashSet::new();
    let candidates = collect_footnote_candidates(body_html);
    let mut notes = Vec::new();
    let mut used_candidates = HashSet::new();
    let mut ordered_ref_targets = Vec::new();

    for target_id in &target_ids {
        let lower = target_id.to_ascii_lowercase();
        if lower.contains("ref") || !seen_target_ids.insert(target_id.clone()) {
            continue;
        }
        ordered_ref_targets.push(target_id.clone());

        let matched = candidates
            .iter()
            .enumerate()
            .find(|(idx, candidate)| {
                !used_candidates.contains(idx) && footnote_candidate_contains_target(candidate, &target_id)
            })
            .map(|(idx, _)| idx)
            .or_else(|| None);

        let Some(candidate_idx) = matched else {
            continue;
        };
        used_candidates.insert(candidate_idx);
        let text = candidates[candidate_idx].text.clone();
        if !is_meaningful_footnote_text(&text) {
            continue;
        }

        notes.push(FootnoteEntry {
            id: target_id.clone(),
            number: notes.len() + 1,
            text,
        });
    }

    // If we have no target-id matches, map refs to candidates strictly by order.
    if notes.is_empty() {
        for (position, target_id) in ordered_ref_targets.iter().enumerate() {
            if let Some(candidate) = candidates.get(position) {
                if is_meaningful_footnote_text(&candidate.text) {
                    notes.push(FootnoteEntry {
                        id: target_id.clone(),
                        number: notes.len() + 1,
                        text: candidate.text.clone(),
                    });
                }
            }
        }
    }

    notes
}

fn collect_footnote_candidates(body_html: &str) -> Vec<FootnoteCandidate> {
    // Substack-specific path: look for <div class="footnote" data-component-name="FootnoteToDOM">
    // These are individual divs (not a section wrapping multiple <li>), each with a
    // <div class="footnote-content"> child holding the actual text.
    let substack_result = collect_substack_footnote_candidates(body_html);
    if !substack_result.is_empty() {
        return substack_result;
    }

    let fragment = Html::parse_fragment(body_html);
    let section_selector =
        Selector::parse("section, div, aside, ol, ul").expect("valid footnote section selector");
    let li_selector = Selector::parse("li").expect("valid li selector");
    let block_selector = Selector::parse("li, p, div").expect("valid footnote block selector");
    let id_selector = Selector::parse("[id]").expect("valid id selector");
    let link_selector = Selector::parse("a[href]").expect("valid link selector");
    let mut result = Vec::new();

    // Collect real container sections (not individual footnote entries like Substack's).
    let mut footnote_sections = Vec::new();
    for section in fragment.select(&section_selector) {
        let tag = section.value().name();
        let id = section.value().attr("id").unwrap_or("").to_ascii_lowercase();
        let class_name = section.value().attr("class").unwrap_or("").to_ascii_lowercase();
        let epub_type = section.value().attr("epub:type").unwrap_or("").to_ascii_lowercase();
        let role = section.value().attr("role").unwrap_or("").to_ascii_lowercase();

        // Skip individual Substack footnote divs; we only want real container sections.
        if tag == "div" && class_name == "footnote" {
            continue;
        }

        let looks_like_section = id.contains("footnote")
            || id.contains("endnote")
            || class_name.contains("footnote")
            || class_name.contains("endnote")
            || epub_type.contains("footnote")
            || epub_type.contains("endnote")
            || role.contains("doc-endnotes");
        if looks_like_section {
            footnote_sections.push(section.html());
        }
    }

    // Preferred path: parse dedicated footnotes/endnotes sections only.
    for section_html in footnote_sections {
        let section_fragment = Html::parse_fragment(&section_html);
        for element in section_fragment.select(&li_selector) {
            let candidate = build_footnote_candidate(
                &element,
                &id_selector,
                &link_selector,
            );
            if let Some(candidate) = candidate {
                result.push(candidate);
            }
        }
    }

    if !result.is_empty() {
        return result;
    }

    // Fallback path: only keep blocks that have explicit backlink markers.
    for element in fragment.select(&block_selector) {
        let inner_html = element.inner_html();
        let lower_inner = inner_html.to_ascii_lowercase();
        if !(lower_inner.contains("footnote-backref")
            || lower_inner.contains("back to content")
            || lower_inner.contains("return to article")
            || lower_inner.contains("return to content")
            || lower_inner.contains("#fnref")
            || lower_inner.contains("href=\"#fn")
            || lower_inner.contains("href=\"#footnote")
            || lower_inner.contains("footnote-number"))
        {
            continue;
        }
        let candidate = build_footnote_candidate(
            &element,
            &id_selector,
            &link_selector,
        );
        if let Some(candidate) = candidate {
            if is_meaningful_footnote_text(&candidate.text) {
                result.push(candidate);
            }
        }
    }

    result
}

/// Substack-specific: each footnote is a `<div class="footnote">` containing
/// `<a class="footnote-number" id="footnote-N-POSTID">N</a>` and
/// `<div class="footnote-content"><p>text</p></div>`.
fn collect_substack_footnote_candidates(body_html: &str) -> Vec<FootnoteCandidate> {
    let fragment = Html::parse_fragment(body_html);
    let div_selector = Selector::parse("div.footnote").expect("valid div.footnote selector");
    let content_selector = Selector::parse(".footnote-content").expect("valid .footnote-content selector");
    let number_selector = Selector::parse("a.footnote-number, a[class*='footnote-number']").expect("valid footnote-number selector");
    let id_selector = Selector::parse("[id]").expect("valid id selector");
    let link_selector = Selector::parse("a[href]").expect("valid link selector");
    let mut result = Vec::new();

    for footnote_div in fragment.select(&div_selector) {
        let class_name = footnote_div.value().attr("class").unwrap_or("");
        // Only match divs whose class is exactly "footnote" (not "footnote-content", etc.)
        if !class_name.split_whitespace().any(|c| c == "footnote") {
            continue;
        }
        // Skip divs that are nested content containers
        if class_name.contains("footnote-content") || class_name.contains("footnote-anchor") {
            continue;
        }

        // Collect IDs from the footnote-number anchor and any [id] elements
        let mut ids = HashSet::new();
        if let Some(id) = footnote_div.value().attr("id").map(str::trim).filter(|v| !v.is_empty()) {
            ids.insert(id.to_string());
        }
        for node in footnote_div.select(&id_selector) {
            if let Some(id) = node.value().attr("id").map(str::trim).filter(|v| !v.is_empty()) {
                ids.insert(id.to_string());
            }
        }
        // Also grab the id from the footnote-number anchor specifically
        for anchor in footnote_div.select(&number_selector) {
            if let Some(id) = anchor.value().attr("id").map(str::trim).filter(|v| !v.is_empty()) {
                ids.insert(id.to_string());
            }
        }

        // Collect href targets
        let mut href_targets = HashSet::new();
        for anchor in footnote_div.select(&link_selector) {
            if let Some(href) = anchor.value().attr("href") {
                if let Some(target) = extract_fragment_id_from_href(href) {
                    if !target.is_empty() {
                        href_targets.insert(target);
                    }
                }
            }
        }

        // Extract text from the footnote-content child
        let text = if let Some(content_div) = footnote_div.select(&content_selector).next() {
            let inner = content_div.inner_html();
            let raw_text = html2text::from_read(inner.as_bytes(), 10_000).unwrap_or(inner);
            cleanup_footnote_text(&raw_text)
        } else {
            // Fallback: extract text from everything except the number anchor
            let cleaned_html = strip_footnote_navigation(&footnote_div.inner_html());
            let raw_text = html2text::from_read(cleaned_html.as_bytes(), 10_000).unwrap_or(cleaned_html);
            cleanup_footnote_text(&raw_text)
        };

        if !is_meaningful_footnote_text(&text) {
            continue;
        }

        result.push(FootnoteCandidate { ids, href_targets, text });
    }

    result
}

fn build_footnote_candidate(
    element: &scraper::element_ref::ElementRef<'_>,
    id_selector: &Selector,
    link_selector: &Selector,
) -> Option<FootnoteCandidate> {
    let tag = element.value().name();
    if tag != "li" {
        let li_selector = Selector::parse("li").expect("valid li selector in candidate builder");
        let li_children = element.select(&li_selector).take(2).count();
        if li_children > 1 {
            return None;
        }
    }

    let mut ids = HashSet::new();
    if let Some(id) = element.value().attr("id").map(str::trim).filter(|v| !v.is_empty()) {
        ids.insert(id.to_string());
    }
    for node in element.select(id_selector) {
        if let Some(id) = node.value().attr("id").map(str::trim).filter(|v| !v.is_empty()) {
            ids.insert(id.to_string());
        }
    }

    let mut href_targets = HashSet::new();
    for anchor in element.select(link_selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(target) = extract_fragment_id_from_href(href) else {
            continue;
        };
        if !target.is_empty() {
            href_targets.insert(target);
        }
    }

    let cleaned_html = strip_footnote_navigation(&element.inner_html());
    let raw_text = html2text::from_read(cleaned_html.as_bytes(), 10_000).unwrap_or(cleaned_html);
    let text = cleanup_footnote_text(&raw_text);
    if !is_meaningful_footnote_text(&text) {
        return None;
    }

    Some(FootnoteCandidate { ids, href_targets, text })
}

fn footnote_candidate_contains_target(candidate: &FootnoteCandidate, target_id: &str) -> bool {
    let target_exact = target_id.to_ascii_lowercase();
    let target_key = normalize_footnote_key(target_id);
    candidate.ids.iter().any(|id| {
        let id_exact = id.to_ascii_lowercase();
        if id_exact == target_exact {
            return true;
        }
        let id_key = normalize_footnote_key(id);
        !id_key.is_empty() && id_key == target_key
    }) || candidate.href_targets.iter().any(|target| {
        let link_exact = target.to_ascii_lowercase();
        if link_exact == target_exact {
            return true;
        }
        let link_key = normalize_footnote_key(target);
        !link_key.is_empty() && link_key == target_key
    })
}

fn strip_footnote_navigation(value: &str) -> String {
    let patterns = [
        r#"(?is)<a[^>]*class=["'][^"']*footnote-backref[^"']*["'][^>]*>.*?</a>"#,
        r#"(?is)<a[^>]*class=["'][^"']*footnote-number[^"']*["'][^>]*>.*?</a>"#,
        r#"(?is)<a[^>]*href=["'][^"']*#(?:fnref|footnote-ref|ref|footnote-anchor)[^"']*["'][^>]*>.*?</a>"#,
        r#"(?is)<a[^>]*id=["'][^"']*(?:fnref|footnote-ref)[^"']*["'][^>]*>.*?</a>"#,
        r#"(?is)<a[^>]*>\s*(?:↩|&#8617;|&larr;|back|return)\s*</a>"#,
    ];
    let mut out = value.to_string();
    for pattern in patterns {
        let regex = Regex::new(pattern).expect("valid footnote navigation regex");
        out = regex.replace_all(&out, "").into_owned();
    }
    out
}

fn looks_like_footnote_id(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    // Match footnote target IDs but not footnote-anchor IDs (those are the refs in body)
    if id.contains("footnote-anchor") {
        return false;
    }
    id.contains("footnote") || id.starts_with("fn") || id.contains("fn-")
}

fn remove_footnote_containers(body_html: &str) -> String {
    // We need to remove footnote containers from raw HTML. Regex with .*? fails on
    // nested elements (e.g. Substack's <div class="footnote"><div class="footnote-content">
    // ...</div></div>).  DOM parsing (Html::parse_fragment) normalizes HTML so serialized
    // output won't match the original source for string replacement.
    //
    // Solution: find opening tags that look like footnote containers, then count nesting
    // depth of that specific tag to find the correct closing tag.
    let tag_patterns: Vec<(String, Regex)> = ["section", "div", "aside", "ol", "ul"]
        .iter()
        .filter_map(|tag| {
            // Match opening tags with footnote/endnote in class, id, or data-component-name
            let pattern = format!(
                r#"(?is)<{tag}(?:\s[^>]*)?\s(?:class|id|data-component-name)=[\"'][^\"']*(?:footnote|endnote|FootnoteToDOM)[^\"']*[\"'][^>]*>"#,
                tag = tag
            );
            Regex::new(&pattern).ok().map(|rx| (tag.to_string(), rx))
        })
        .collect();

    let mut out = body_html.to_string();
    for (tag, open_regex) in &tag_patterns {
        loop {
            let Some(m) = open_regex.find(&out) else {
                break;
            };

            // Check if this is a footnote-content or footnote-anchor div (skip those,
            // they are children and will be removed with their parent).
            let matched_tag_text = m.as_str().to_ascii_lowercase();
            if tag == "div"
                && (matched_tag_text.contains("footnote-content")
                    || matched_tag_text.contains("footnote-anchor"))
            {
                // Can't just break — there may be more matches after this one.
                // Replace this specific opening tag with a placeholder, then restore later.
                // Simpler: just skip past this match by working on the tail.
                // But since we're modifying `out` in a loop, let's just skip this specific
                // class by requiring exact "footnote" class word for divs.
                let class_attr = extract_attr_value(&matched_tag_text, "class").unwrap_or_default();
                if !class_attr.split_whitespace().any(|c| c == "footnote" || c == "footnotes")
                    && !matched_tag_text.contains("footnotetoddom")
                {
                    // This match is only footnote-content / footnote-anchor, skip it
                    // by removing this iteration's match from consideration.
                    // We break to avoid infinite loop; the regex will keep matching.
                    break;
                }
            }

            let start = m.start();
            let after_open = m.end();
            let closing_tag = format!("</{}>", tag);
            let opening_prefix = format!("<{}", tag);

            // Walk forward counting nesting depth to find the matching close tag.
            let rest = &out[after_open..];
            let mut depth = 1i32;
            let mut cursor = 0usize;
            while depth > 0 && cursor < rest.len() {
                // Find next opening or closing tag of same type
                let next_open = rest[cursor..].find(&opening_prefix).map(|p| p + cursor);
                let ci = rest[cursor..].to_ascii_lowercase();
                let next_close = ci.find(&closing_tag).map(|p| p + cursor);

                match (next_open, next_close) {
                    (Some(o), Some(c)) if o < c => {
                        // Check the open tag is actually a tag (followed by space, >, or /)
                        let after = rest.as_bytes().get(o + opening_prefix.len());
                        if after.map(|b| *b == b' ' || *b == b'>' || *b == b'/' || *b == b'\n' || *b == b'\r' || *b == b'\t').unwrap_or(false) {
                            depth += 1;
                        }
                        cursor = o + 1;
                    }
                    (_, Some(c)) => {
                        depth -= 1;
                        if depth == 0 {
                            let end = after_open + c + closing_tag.len();
                            out = format!("{}{}", &out[..start], &out[end..]);
                            break;
                        }
                        cursor = c + closing_tag.len();
                    }
                    (Some(o), None) => {
                        cursor = o + 1;
                    }
                    (None, None) => {
                        break;
                    }
                }
            }

            // Safety: if we didn't manage to remove anything, break to avoid infinite loop.
            if depth != 0 {
                break;
            }
        }
    }
    out
}

/// Extract a specific attribute value from a raw HTML opening-tag string.
/// e.g. `extract_attr_value(r#"<div class="foo bar">"#, "class")` -> Some("foo bar")
fn extract_attr_value(tag_html: &str, attr_name: &str) -> Option<String> {
    let pattern = format!(r#"(?i){}=[\"']([^\"']*)[\"']"#, regex::escape(attr_name));
    let rx = Regex::new(&pattern).ok()?;
    rx.captures(tag_html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn replace_footnote_refs_with_tokens(input_html: &str, footnotes: &[FootnoteEntry]) -> String {
    let mut by_id = HashMap::new();
    for note in footnotes {
        by_id.insert(note.id.to_ascii_lowercase(), note.number);
        let normalized = normalize_footnote_key(&note.id);
        if !normalized.is_empty() {
            by_id.entry(normalized).or_insert(note.number);
        }
    }

    let anchor_regex =
        Regex::new(r#"(?is)<a[^>]*href=["']([^"']+)["'][^>]*>.*?</a>"#).expect("valid anchor-footnote regex");
    anchor_regex
        .replace_all(input_html, |caps: &regex::Captures| {
            let href = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let Some(target_id) = extract_fragment_id_from_href(href) else {
                return caps.get(0).map(|m| m.as_str()).unwrap_or_default().to_string();
            };
            if !looks_like_footnote_id(&target_id) {
                return caps.get(0).map(|m| m.as_str()).unwrap_or_default().to_string();
            }
            let lookup = target_id.to_ascii_lowercase();
            by_id
                .get(&lookup)
                .copied()
                .or_else(|| {
                    let normalized = normalize_footnote_key(&target_id);
                    if normalized.is_empty() {
                        None
                    } else {
                        by_id.get(&normalized).copied()
                    }
                })
                .map(|number| format!("[[FN:{number}]]"))
                .unwrap_or_else(|| caps.get(0).map(|m| m.as_str()).unwrap_or_default().to_string())
        })
        .into_owned()
}

fn normalize_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_plain_text(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n");
    let mut out = String::new();
    let mut blank_run = 0usize;
    for raw_line in normalized.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn cleanup_footnote_text(value: &str) -> String {
    let mut text = normalize_whitespace(value);
    let leading = Regex::new(r#"^\s*(?:\[\d+\]|\d+\s*[\.\)])\s*"#).expect("valid leading-footnote regex");
    text = leading.replace(&text, "").to_string();
    let trailing = Regex::new(
        r#"(?i)(↩|&#8617;|&#x21a9;|&larr;|back(?: to (?:content|article|text))?|\[back\]|return to (?:article|content))\s*$"#,
    )
    .expect("valid trailing-footnote regex");
    while trailing.is_match(&text) {
        text = trailing.replace(&text, "").to_string();
        text = text.trim().to_string();
    }
    text.trim().to_string()
}

fn is_meaningful_footnote_text(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let normalized = lower
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .trim()
        .to_string();
    if normalized.is_empty() {
        return false;
    }
    let trivial = [
        "back",
        "return",
        "back to content",
        "return to article",
        "return to content",
        "see above",
    ];
    if trivial.contains(&normalized.as_str()) {
        return false;
    }
    !normalized.chars().all(|ch| ch.is_ascii_digit())
}

fn normalize_footnote_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn collect_footnote_target_ids(body_html: &str) -> Vec<String> {
    let regex = Regex::new(r#"(?is)<a[^>]*href=["']([^"']+)["'][^>]*>"#).expect("valid footnote-target regex");
    // Also detect Substack footnote-anchor links by class
    let class_regex = Regex::new(r#"(?is)<a[^>]*class=["'][^"']*footnote-anchor[^"']*["'][^>]*href=["']([^"']+)["'][^>]*>"#).expect("valid footnote-anchor regex");
    let class_regex_alt = Regex::new(r#"(?is)<a[^>]*href=["']([^"']+)["'][^>]*class=["'][^"']*footnote-anchor[^"']*["'][^>]*>"#).expect("valid footnote-anchor-alt regex");

    let mut result = Vec::new();
    let mut seen = HashSet::new();

    // Collect from Substack footnote-anchor links first (they always point to footnotes)
    for regex_instance in [&class_regex, &class_regex_alt] {
        for captures in regex_instance.captures_iter(body_html) {
            let Some(href_match) = captures.get(1) else {
                continue;
            };
            let Some(id) = extract_fragment_id_from_href(href_match.as_str()) else {
                continue;
            };
            if !id.is_empty() && seen.insert(id.clone()) {
                result.push(id);
            }
        }
    }

    // Standard footnote href detection
    for captures in regex.captures_iter(body_html) {
        let Some(href_match) = captures.get(1) else {
            continue;
        };
        let Some(id) = extract_fragment_id_from_href(href_match.as_str()) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let lower = id.to_ascii_lowercase();
        if !(lower.contains("footnote") || lower.starts_with("fn")) {
            continue;
        }
        // Skip "footnote-anchor" IDs — those are the refs, not the targets
        if lower.contains("footnote-anchor") {
            continue;
        }
        if seen.insert(id.clone()) {
            result.push(id);
        }
    }
    result
}

fn extract_fragment_id_from_href(href: &str) -> Option<String> {
    // Handle both fragment-only (#foo) and full-URL (https://...#foo) hrefs
    let hash_pos = href.rfind('#')?;
    let raw = href[hash_pos + 1..].trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.to_string())
}

fn render_plain_text(html_with_markers: &str, footnotes: &[FootnoteEntry]) -> String {
    let with_break_hints = add_block_break_hints(html_with_markers);
    let anchor_strip = Regex::new(r"(?is)<a\b[^>]*>(.*?)</a>").expect("valid anchor-strip regex");
    let stripped = anchor_strip.replace_all(&with_break_hints, "$1").into_owned();
    let raw_text = html2text::from_read(stripped.as_bytes(), 10_000).unwrap_or(stripped);
    let normalized_main = normalize_plain_text(&raw_text);
    inject_text_footnotes(&normalized_main, footnotes)
}

fn add_block_break_hints(value: &str) -> String {
    let regex = Regex::new(r#"(?i)</(p|div|li|blockquote|h1|h2|h3|h4|h5|h6|section|article)>"#)
        .expect("valid block-break regex");
    let with_blocks = regex.replace_all(value, "$0\n\n").into_owned();
    let br_regex = Regex::new(r#"(?i)<br\s*/?>"#).expect("valid br regex");
    br_regex.replace_all(&with_blocks, "<br/>\n").into_owned()
}

fn inject_text_footnotes(main_text: &str, footnotes: &[FootnoteEntry]) -> String {
    let mut out = main_text.to_string();
    for note in footnotes {
        out = out.replace(&format!("[[FN:{}]]", note.number), &format!("[{}]", note.number));
    }
    if footnotes.is_empty() {
        return out;
    }

    out.push_str("\n\nFootnotes\n");
    for note in footnotes {
        out.push_str(&format!("[{}] {}\n", note.number, note.text));
    }
    out.trim_end().to_string()
}

fn build_epub_body(html_with_markers: &str, footnotes: &[FootnoteEntry]) -> String {
    let mut body = sanitize_html_for_epub(html_with_markers);

    for note in footnotes {
        let token = format!("[[FN:{}]]", note.number);
        let marker = format!(
            r##"<a class="footnote-ref" href="#footnote-{}" id="footnote-ref-{}" epub:type="noteref"><sup class="footnote-ref-num">{}</sup></a>"##,
            note.number, note.number, note.number
        );
        body = body.replace(&token, &marker);
    }

    if !contains_block_markup(&body) {
        let plain = normalize_plain_text(
            &html2text::from_read(body.as_bytes(), 10_000).unwrap_or_else(|_| body.clone()),
        );
        let fallback = plain
            .split("\n\n")
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .map(|chunk| format!("<p>{}</p>", crate::utils::escape_xml(chunk).replace('\n', "<br/>")))
            .collect::<Vec<_>>();
        body = if fallback.is_empty() {
            "<p>No content available.</p>".to_string()
        } else {
            fallback.join("\n    ")
        };
    }

    if footnotes.is_empty() {
        return body;
    }

    let mut indexed = HashMap::new();
    for note in footnotes {
        indexed.insert(note.number, note);
    }

    body.push_str("\n    <section class=\"footnotes\">");
    body.push_str("\n      <h2>Footnotes</h2>");
    body.push_str("\n      <ol>");
    for number in 1..=footnotes.len() {
        if let Some(note) = indexed.get(&number) {
            body.push_str(&format!(
                "\n        <li id=\"footnote-{}\">{} <a class=\"footnote-backref\" href=\"#footnote-ref-{}\" epub:type=\"backlink\">[back]</a></li>",
                note.number,
                crate::utils::escape_xml(&note.text),
                note.number
            ));
        }
    }
    body.push_str("\n      </ol>");
    body.push_str("\n    </section>");
    body
}

fn sanitize_html_for_epub(value: &str) -> String {
    let strip_media = Regex::new(r#"(?is)<(script|style|iframe|video|audio)[^>]*>.*?</(script|style|iframe|video|audio)>"#)
        .expect("valid strip-media regex");
    let mut out = strip_media.replace_all(value, "").into_owned();
    let br_regex = Regex::new(r#"(?i)<br\s*>"#).expect("valid br normalize regex");
    out = br_regex.replace_all(&out, "<br/>").into_owned();
    let hr_regex = Regex::new(r#"(?i)<hr([^>/]*?)>"#).expect("valid hr normalize regex");
    out = hr_regex.replace_all(&out, "<hr$1/>").into_owned();
    let img_regex = Regex::new(r#"(?i)<img([^>/]*?)>"#).expect("valid img normalize regex");
    out = img_regex.replace_all(&out, "<img$1/>").into_owned();
    out
}

fn contains_block_markup(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("<p") || lower.contains("<div") || lower.contains("<section") || lower.contains("<blockquote")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footnote_text_comes_from_container_not_backlink_only() {
        let body = r##"
<p>Body text<a href="#footnote-1">1</a></p>
<section class="footnotes">
  <ol>
    <li>
      <a id="footnote-1"></a>
      This is the real footnote text.
      <a href="#fnref-1" class="footnote-backref">back</a>
    </li>
  </ol>
</section>
"##;
        let notes = extract_footnotes(body);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("real footnote text"));
        assert_ne!(notes[0].text.to_ascii_lowercase(), "back");
    }

    #[test]
    fn author_falls_back_to_json_ld_name() {
        let html = r##"
<html>
  <head>
    <script type="application/ld+json">
      {"@context":"https://schema.org","@type":"NewsArticle","author":{"@type":"Person","name":"Dara Chaw"}}
    </script>
  </head>
  <body></body>
</html>
"##;
        let document = Html::parse_document(html);
        let author = extract_author(&document, html);
        assert_eq!(author.as_deref(), Some("Dara Chaw"));
    }

    #[test]
    fn substack_footnote_div_structure_is_extracted() {
        let body = r##"
<p>Some body text<a data-component-name="FootnoteAnchorToDOM" id="footnote-anchor-1-999" href="https://example.substack.com/p/test#footnote-1-999" target="_self" class="footnote-anchor">1</a> and more text<a data-component-name="FootnoteAnchorToDOM" id="footnote-anchor-2-999" href="https://example.substack.com/p/test#footnote-2-999" target="_self" class="footnote-anchor">2</a></p>
<div data-component-name="FootnoteToDOM" class="footnote">
  <a id="footnote-1-999" href="https://example.substack.com/p/test#footnote-anchor-1-999" contenteditable="false" class="footnote-number">1</a>
  <div class="footnote-content">
    <p><span>This is the first footnote with actual content.</span></p>
  </div>
</div>
<div data-component-name="FootnoteToDOM" class="footnote">
  <a id="footnote-2-999" href="https://example.substack.com/p/test#footnote-anchor-2-999" contenteditable="false" class="footnote-number">2</a>
  <div class="footnote-content">
    <p><span>Second footnote text here.</span></p>
  </div>
</div>
"##;
        let notes = extract_footnotes(body);
        assert_eq!(notes.len(), 2, "Should extract 2 footnotes, got {:?}", notes);
        assert!(
            notes[0].text.contains("first footnote"),
            "First footnote text should contain content, got: {}",
            notes[0].text
        );
        assert!(
            notes[1].text.contains("Second footnote"),
            "Second footnote text should contain content, got: {}",
            notes[1].text
        );

        // Verify the full pipeline produces output with footnote markers
        let processed = process_body_for_exports(body);
        assert!(
            processed.plain_text.contains("[1]"),
            "Plain text should contain footnote reference [1], got: {}",
            processed.plain_text
        );
        assert!(
            processed.plain_text.contains("Footnotes"),
            "Plain text should contain Footnotes section, got: {}",
            processed.plain_text
        );
        assert!(
            processed.plain_text.contains("first footnote"),
            "Plain text footnote section should contain footnote text, got: {}",
            processed.plain_text
        );

        // Verify exactly ONE Footnotes section (no duplicates!)
        let footnote_section_count = processed.plain_text.matches("Footnotes").count();
        assert_eq!(
            footnote_section_count, 1,
            "Should have exactly 1 Footnotes section, got {}. Full text:\n{}",
            footnote_section_count, processed.plain_text
        );

        // Same check for epub body: only one <section class="footnotes">
        let epub_footnote_sections = processed.epub_body.matches("class=\"footnotes\"").count();
        assert!(
            epub_footnote_sections <= 1,
            "EPUB should have at most 1 footnotes section, got {}. Full body:\n{}",
            epub_footnote_sections, processed.epub_body
        );

        // Verify the original footnote-content divs are removed from the body
        assert!(
            !processed.epub_body.contains("footnote-content"),
            "EPUB body should not contain original footnote-content divs. Full body:\n{}",
            processed.epub_body
        );
    }

    #[test]
    fn extract_fragment_from_full_url() {
        let result = extract_fragment_id_from_href("https://example.substack.com/p/test#footnote-1-999");
        assert_eq!(result, Some("footnote-1-999".to_string()));
    }

    #[test]
    fn extract_fragment_from_hash_only() {
        let result = extract_fragment_id_from_href("#footnote-1");
        assert_eq!(result, Some("footnote-1".to_string()));
    }
}
