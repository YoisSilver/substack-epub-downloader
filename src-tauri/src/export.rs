use crate::models::{
    CoverMode, ExportFailure, ExportFormat, ExportJobRequest, ExportJobResult, ExportMode, Granularity, MetadataField,
    OrderMode, PostContent, PostSummary, SortDirection,
};
use crate::substack::{build_http_client, fetch_bytes_with_retries, fetch_post_content};
use crate::utils::{
    decode_data_url, escape_xml, media_type_to_extension, parse_datetime_flexible, sanitize_filename,
};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use image::ImageFormat;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::FileOptions;
use zip::ZipWriter;

const RETRIES_PER_REQUEST: usize = 3;

#[derive(Debug, Clone)]
struct CoverAsset {
    bytes: Vec<u8>,
    media_type: String,
    extension: String,
}

pub async fn run_export_job(request: ExportJobRequest) -> Result<ExportJobResult> {
    if request.formats.is_empty() {
        return Err(anyhow!("At least one format must be selected."));
    }
    let output_dir = PathBuf::from(request.output_dir.trim());
    if output_dir.as_os_str().is_empty() {
        return Err(anyhow!("Output directory is required."));
    }
    fs::create_dir_all(&output_dir).context("Failed to create output directory.")?;

    let selected = select_posts(&request)?;
    if selected.is_empty() {
        return Err(anyhow!("No posts matched the current selection."));
    }
    let ordered = order_posts(selected, &request.order_mode, &request.manual_order, &request.sort_direction);

    let client = build_http_client()?;
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    let mut warnings = Vec::new();
    let mut contents = Vec::new();

    for summary in ordered {
        match fetch_post_content(&client, &summary, RETRIES_PER_REQUEST).await {
            Ok(content) => {
                succeeded.push(content.summary.id.clone());
                contents.push(content);
            }
            Err(error) => {
                failed.push(ExportFailure {
                    post_id: summary.id,
                    reason: error.to_string(),
                });
            }
        }
    }

    if contents.is_empty() {
        return Err(anyhow!("All post downloads failed; no output generated."));
    }

    let metadata_fields: HashSet<MetadataField> = request.metadata_fields.iter().cloned().collect();
    let cover_asset = if request.formats.contains(&ExportFormat::Epub) {
        match resolve_cover(&request, &client).await {
            Ok(cover) => cover,
            Err(error) => {
                warnings.push(format!("Cover setup issue: {error}"));
                None
            }
        }
    } else {
        None
    };

    let mut output_files = Vec::new();
    if request.formats.contains(&ExportFormat::Txt) {
        output_files.extend(write_txt_outputs(
            &output_dir,
            &request.publication_title,
            &contents,
            &metadata_fields,
            &request.granularity,
        )?);
    }
    if request.formats.contains(&ExportFormat::Epub) {
        output_files.extend(write_epub_outputs(
            &output_dir,
            &request.publication_title,
            request.publication_author.as_deref().unwrap_or("Unknown author"),
            &contents,
            &metadata_fields,
            &request.granularity,
            cover_asset.as_ref(),
        )?);
    }

    Ok(ExportJobResult {
        succeeded,
        failed,
        output_files,
        warnings,
    })
}

fn select_posts(request: &ExportJobRequest) -> Result<Vec<PostSummary>> {
    match request.mode {
        ExportMode::EntireProfile => Ok(request.posts.clone()),
        ExportMode::SpecificPosts => {
            if request.selected_post_ids.is_empty() {
                return Err(anyhow!("No specific posts selected."));
            }
            let selected_ids: HashSet<&String> = request.selected_post_ids.iter().collect();
            let selected: Vec<PostSummary> = request
                .posts
                .iter()
                .filter(|post| selected_ids.contains(&post.id))
                .cloned()
                .collect();
            Ok(selected)
        }
    }
}

fn order_posts(posts: Vec<PostSummary>, order_mode: &OrderMode, manual_order: &[String], sort_direction: &SortDirection) -> Vec<PostSummary> {
    let mut ordered = posts;
    match order_mode {
        OrderMode::Manual if !manual_order.is_empty() => {
            let mut by_id: HashMap<String, PostSummary> = ordered
                .into_iter()
                .map(|post| (post.id.clone(), post))
                .collect();
            let mut manual_sorted = Vec::with_capacity(by_id.len());
            for id in manual_order {
                if let Some(post) = by_id.remove(id) {
                    manual_sorted.push(post);
                }
            }
            let mut remaining: Vec<PostSummary> = by_id.into_values().collect();
            remaining.sort_by(|a, b| compare_post_dates(a, b, sort_direction));
            manual_sorted.extend(remaining);
            ordered = manual_sorted;
        }
        _ => {
            ordered.sort_by(|a, b| compare_post_dates(a, b, sort_direction));
        }
    }
    ordered
}

fn compare_post_dates(a: &PostSummary, b: &PostSummary, sort_direction: &SortDirection) -> std::cmp::Ordering {
    let a_ts = parse_datetime_flexible(&a.published_at)
        .map(|value| value.timestamp_millis())
        .unwrap_or(0);
    let b_ts = parse_datetime_flexible(&b.published_at)
        .map(|value| value.timestamp_millis())
        .unwrap_or(0);
    let primary = match sort_direction {
        SortDirection::Desc => b_ts.cmp(&a_ts),
        SortDirection::Asc => a_ts.cmp(&b_ts),
    };
    if primary == std::cmp::Ordering::Equal {
        a.title.to_lowercase().cmp(&b.title.to_lowercase())
    } else {
        primary
    }
}

async fn resolve_cover(request: &ExportJobRequest, client: &reqwest::Client) -> Result<Option<CoverAsset>> {
    match request.cover_mode {
        CoverMode::Custom => {
            let Some(data_url) = request.custom_cover_data_url.as_deref() else {
                return Err(anyhow!("Custom cover mode selected but no file uploaded."));
            };
            let (bytes, mime_hint) = decode_data_url(data_url)?;
            Ok(Some(normalize_cover_asset(bytes, Some(mime_hint))?))
        }
        CoverMode::SubstackAuthor => {
            let Some(cover_url) = request.author_cover_url.as_deref() else {
                return Ok(None);
            };
            let bytes = fetch_bytes_with_retries(client, cover_url, RETRIES_PER_REQUEST).await?;
            Ok(Some(normalize_cover_asset(bytes, None)?))
        }
    }
}

fn normalize_cover_asset(bytes: Vec<u8>, mime_hint: Option<String>) -> Result<CoverAsset> {
    if bytes.is_empty() {
        return Err(anyhow!("Cover image bytes are empty."));
    }
    let guessed = image::guess_format(&bytes).ok();
    let (media_type, extension) = if let Some(format) = guessed {
        match format {
            ImageFormat::Png => ("image/png".to_string(), "png".to_string()),
            ImageFormat::Jpeg => ("image/jpeg".to_string(), "jpg".to_string()),
            ImageFormat::Gif => ("image/gif".to_string(), "gif".to_string()),
            ImageFormat::WebP => ("image/webp".to_string(), "webp".to_string()),
            _ => {
                let mime = mime_hint.unwrap_or_else(|| "image/jpeg".to_string());
                (mime.clone(), media_type_to_extension(&mime).to_string())
            }
        }
    } else {
        let mime = mime_hint.unwrap_or_else(|| "image/jpeg".to_string());
        (mime.clone(), media_type_to_extension(&mime).to_string())
    };

    Ok(CoverAsset {
        bytes,
        media_type,
        extension,
    })
}

fn write_txt_outputs(
    output_dir: &Path,
    publication_title: &str,
    posts: &[PostContent],
    metadata_fields: &HashSet<MetadataField>,
    granularity: &Granularity,
) -> Result<Vec<String>> {
    match granularity {
        Granularity::PerPost => posts
            .iter()
            .map(|post| {
                let filename = format!(
                    "{} - {}.txt",
                    sanitize_filename(publication_title),
                    sanitize_filename(&post.summary.title)
                );
                let file_path = output_dir.join(filename);
                let content = render_txt_post(post, metadata_fields);
                fs::write(&file_path, content).context("Failed writing TXT file.")?;
                Ok(file_path.to_string_lossy().to_string())
            })
            .collect(),
        Granularity::Combined => {
            let filename = format!("{} - combined.txt", sanitize_filename(publication_title));
            let file_path = output_dir.join(filename);
            let mut combined = String::new();
            combined.push_str(&format!("Publication: {}\n", publication_title));
            combined.push_str(&format!("Generated: {}\n\n", Utc::now().to_rfc3339()));

            for post in posts {
                combined.push_str("============================================================\n");
                combined.push_str(&render_txt_post(post, metadata_fields));
                combined.push('\n');
            }
            fs::write(&file_path, combined).context("Failed writing combined TXT file.")?;
            Ok(vec![file_path.to_string_lossy().to_string()])
        }
    }
}

fn render_txt_post(post: &PostContent, metadata_fields: &HashSet<MetadataField>) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}\n", post.summary.title));
    out.push_str("------------------------------------------------------------\n");
    out.push_str(&render_metadata_lines(post, metadata_fields));
    out.push('\n');
    out.push_str(post.plain_text.trim());
    out.push('\n');
    out
}

fn render_metadata_lines(post: &PostContent, metadata_fields: &HashSet<MetadataField>) -> String {
    let mut fields = Vec::new();
    if metadata_fields.contains(&MetadataField::Title) {
        fields.push(format!("Title: {}", post.summary.title));
    }
    if metadata_fields.contains(&MetadataField::Author) {
        fields.push(format!(
            "Author: {}",
            post.summary.author.as_deref().unwrap_or("Unknown")
        ));
    }
    if metadata_fields.contains(&MetadataField::PublishedAt) {
        fields.push(format!("Published: {}", post.summary.published_at));
    }
    if metadata_fields.contains(&MetadataField::Url) {
        fields.push(format!("URL: {}", post.summary.url));
    }
    if metadata_fields.contains(&MetadataField::Tags) {
        let tags = post
            .summary
            .tags
            .as_ref()
            .map(|items| items.join(", "))
            .unwrap_or_else(|| "N/A".to_string());
        fields.push(format!("Tags: {tags}"));
    }
    if metadata_fields.contains(&MetadataField::Subtitle) {
        fields.push(format!(
            "Subtitle: {}",
            post.summary.subtitle.as_deref().unwrap_or("N/A")
        ));
    }
    if metadata_fields.contains(&MetadataField::ReadingTime) {
        fields.push(format!(
            "Reading time: {}",
            post.reading_time_minutes
                .map(|v| format!("{v} min"))
                .unwrap_or_else(|| "N/A".to_string())
        ));
    }
    if metadata_fields.contains(&MetadataField::Summary) {
        fields.push(format!(
            "Summary: {}",
            post.summary_text.as_deref().unwrap_or("N/A")
        ));
    }
    fields.join("\n")
}

fn write_epub_outputs(
    output_dir: &Path,
    publication_title: &str,
    publication_author: &str,
    posts: &[PostContent],
    metadata_fields: &HashSet<MetadataField>,
    granularity: &Granularity,
    cover: Option<&CoverAsset>,
) -> Result<Vec<String>> {
    match granularity {
        Granularity::PerPost => posts
            .iter()
            .map(|post| {
                let filename = format!(
                    "{} - {}.epub",
                    sanitize_filename(publication_title),
                    sanitize_filename(&post.summary.title)
                );
                let file_path = output_dir.join(filename);
                write_epub(
                    &file_path,
                    &post.summary.title,
                    post.summary.author.as_deref().unwrap_or(publication_author),
                    std::slice::from_ref(post),
                    metadata_fields,
                    cover,
                )?;
                Ok(file_path.to_string_lossy().to_string())
            })
            .collect(),
        Granularity::Combined => {
            let filename = format!("{} - combined.epub", sanitize_filename(publication_title));
            let file_path = output_dir.join(filename);
            write_epub(
                &file_path,
                publication_title,
                publication_author,
                posts,
                metadata_fields,
                cover,
            )?;
            Ok(vec![file_path.to_string_lossy().to_string()])
        }
    }
}

fn write_epub(
    output_file: &Path,
    book_title: &str,
    book_author: &str,
    posts: &[PostContent],
    metadata_fields: &HashSet<MetadataField>,
    cover: Option<&CoverAsset>,
) -> Result<()> {
    let file = File::create(output_file).context("Failed to create EPUB file.")?;
    let mut zip = ZipWriter::new(file);

    let stored = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    zip.start_file("META-INF/container.xml", deflated)?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
    )?;

    let mut manifest_items = Vec::new();
    let mut spine_items = Vec::new();

    manifest_items.push(r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#.to_string());

    if let Some(cover) = cover {
        let cover_path = format!("OEBPS/images/cover.{}", cover.extension);
        zip.start_file(cover_path, deflated)?;
        zip.write_all(&cover.bytes)?;
        manifest_items.push(format!(
            r#"<item id="cover-image" href="images/cover.{}" media-type="{}" properties="cover-image"/>"#,
            cover.extension, cover.media_type
        ));
        manifest_items.push(r#"<item id="cover-page" href="text/cover.xhtml" media-type="application/xhtml+xml"/>"#.to_string());
        spine_items.push(r#"<itemref idref="cover-page"/>"#.to_string());
    }

    for (index, _post) in posts.iter().enumerate() {
        let chapter_id = format!("chapter-{}", index + 1);
        manifest_items.push(format!(
            r#"<item id="{chapter_id}" href="text/{chapter_id}.xhtml" media-type="application/xhtml+xml"/>"#
        ));
        spine_items.push(format!(r#"<itemref idref="{chapter_id}"/>"#));
    }

    zip.start_file("OEBPS/content.opf", deflated)?;
    let identifier = Uuid::new_v4();
    let metadata_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:{identifier}</dc:identifier>
    <dc:title>{}</dc:title>
    <dc:creator>{}</dc:creator>
    <dc:language>en</dc:language>
    <dc:date>{}</dc:date>
  </metadata>
  <manifest>
    {}
  </manifest>
  <spine>
    {}
  </spine>
</package>"#,
        escape_xml(book_title),
        escape_xml(book_author),
        Utc::now().to_rfc3339(),
        manifest_items.join("\n    "),
        spine_items.join("\n    ")
    );
    zip.write_all(metadata_xml.as_bytes())?;

    zip.start_file("OEBPS/nav.xhtml", deflated)?;
    let mut nav_links = Vec::new();
    if cover.is_some() {
        nav_links.push(r#"<li><a href="text/cover.xhtml">Cover</a></li>"#.to_string());
    }
    for (index, post) in posts.iter().enumerate() {
        let chapter_id = format!("chapter-{}", index + 1);
        nav_links.push(format!(
            r#"<li><a href="text/{chapter_id}.xhtml">{}</a></li>"#,
            escape_xml(&post.summary.title)
        ));
    }
    let nav = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Navigation</title></head>
<body>
  <nav epub:type="toc" id="toc" xmlns:epub="http://www.idpf.org/2007/ops">
    <h1>{}</h1>
    <ol>
      {}
    </ol>
  </nav>
</body>
</html>"#,
        escape_xml(book_title),
        nav_links.join("\n      ")
    );
    zip.write_all(nav.as_bytes())?;

    if let Some(cover) = cover {
        zip.start_file("OEBPS/text/cover.xhtml", deflated)?;
        let cover_page = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
  <title>Cover</title>
  <style>
    body {{ text-align: center; font-family: sans-serif; }}
    img {{ max-width: 95%; max-height: 70vh; margin-top: 1rem; }}
    h1 {{ margin-top: 1.5rem; }}
  </style>
</head>
<body>
  <h1>{}</h1>
  <h2>{}</h2>
  <img src="../images/cover.{}" alt="Cover image" />
</body>
</html>"#,
            escape_xml(book_title),
            escape_xml(book_author),
            cover.extension
        );
        zip.write_all(cover_page.as_bytes())?;
    }

    for (index, post) in posts.iter().enumerate() {
        let chapter_id = format!("chapter-{}", index + 1);
        zip.start_file(format!("OEBPS/text/{chapter_id}.xhtml"), deflated)?;
        let chapter_markup = render_epub_chapter(post, metadata_fields);
        zip.write_all(chapter_markup.as_bytes())?;
    }

    zip.finish()?;
    Ok(())
}

fn render_epub_chapter(post: &PostContent, metadata_fields: &HashSet<MetadataField>) -> String {
    let title = escape_xml(&post.summary.title);
    let metadata = render_epub_metadata(post, metadata_fields);
    let body = &post.epub_body;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
  <title>{title}</title>
  <style>
    body {{ font-family: Georgia, \"Times New Roman\", serif; line-height: 1.78; font-size: 1.05rem; color: #202020; }}
    .meta {{ background: #f4f4f4; border: 1px solid #ddd; padding: 0.75rem; margin-bottom: 1rem; }}
    .meta p {{ margin: 0.2rem 0; font-size: 0.92rem; }}
    section p {{ margin: 0 0 1.25em; }}
    section h2, section h3 {{ margin-top: 1.7em; margin-bottom: 0.7em; }}
    section ul, section ol {{ margin: 0.5em 0 1.25em 1.2em; }}
    section li {{ margin-bottom: 0.5em; }}
    section blockquote {{ margin: 1.2em 0; padding-left: 1em; border-left: 3px solid #cfd5e2; color: #444; }}
    .footnote-ref {{ text-decoration: none; line-height: 0; }}
    .footnote-ref-num {{ font-size: 0.72em; vertical-align: super; }}
    .footnotes {{ border-top: 1px solid #ddd; margin-top: 2em; padding-top: 1em; }}
    .footnotes li {{ margin-bottom: 0.6em; }}
    .footnote-backref {{ text-decoration: none; font-size: 0.9em; }}
  </style>
</head>
<body>
  <h1>{title}</h1>
  <section class="meta">
    {metadata}
  </section>
  <section>
    {body}
  </section>
</body>
</html>"#
    )
}

fn render_epub_metadata(post: &PostContent, metadata_fields: &HashSet<MetadataField>) -> String {
    let mut lines = Vec::new();
    if metadata_fields.contains(&MetadataField::Author) {
        lines.push(format!(
            "<p><strong>Author:</strong> {}</p>",
            escape_xml(post.summary.author.as_deref().unwrap_or("Unknown"))
        ));
    }
    if metadata_fields.contains(&MetadataField::PublishedAt) {
        lines.push(format!(
            "<p><strong>Published:</strong> {}</p>",
            escape_xml(&post.summary.published_at)
        ));
    }
    if metadata_fields.contains(&MetadataField::Url) {
        lines.push(format!(
            "<p><strong>URL:</strong> {}</p>",
            escape_xml(&post.summary.url)
        ));
    }
    if metadata_fields.contains(&MetadataField::Tags) {
        lines.push(format!(
            "<p><strong>Tags:</strong> {}</p>",
            escape_xml(
                &post
                    .summary
                    .tags
                    .as_ref()
                    .map(|items| items.join(", "))
                    .unwrap_or_else(|| "N/A".to_string())
            )
        ));
    }
    if metadata_fields.contains(&MetadataField::Subtitle) {
        lines.push(format!(
            "<p><strong>Subtitle:</strong> {}</p>",
            escape_xml(post.summary.subtitle.as_deref().unwrap_or("N/A"))
        ));
    }
    if metadata_fields.contains(&MetadataField::ReadingTime) {
        lines.push(format!(
            "<p><strong>Reading time:</strong> {}</p>",
            escape_xml(
                &post
                    .reading_time_minutes
                    .map(|v| format!("{v} min"))
                    .unwrap_or_else(|| "N/A".to_string())
            )
        ));
    }
    if metadata_fields.contains(&MetadataField::Summary) {
        lines.push(format!(
            "<p><strong>Summary:</strong> {}</p>",
            escape_xml(post.summary_text.as_deref().unwrap_or("N/A"))
        ));
    }
    if lines.is_empty() {
        "<p>No metadata selected.</p>".to_string()
    } else {
        lines.join("\n    ")
    }
}
