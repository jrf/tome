use anyhow::{Context, Result};
use scraper::{Html, Selector};
use std::io::Read;
use std::time::Duration;
use url::Url;

use crate::metadata;
use crate::model::Bookmark;

const USER_AGENT: &str = "Mozilla/5.0 (compatible; tome/0.1; +https://github.com/jrf/tome)";

const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
const PREVIEW_MAX_WIDTH: u32 = 800;
const PREVIEW_JPEG_QUALITY: u8 = 80;

pub struct FetchResult {
    pub bookmark: Bookmark,
    pub image_url: Option<String>,
    pub html: String,
}

pub fn fetch_url(input: &str) -> Result<FetchResult> {
    let parsed = Url::parse(input).with_context(|| format!("Invalid URL: {}", input))?;
    let canonical = parsed.as_str().to_string();
    let site = parsed.host_str().unwrap_or("").to_string();

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()?;

    let body = client
        .get(&canonical)
        .send()
        .with_context(|| format!("Failed to fetch {}", canonical))?
        .text()
        .context("Failed to read response body")?;

    let (mut bookmark, raw_image) = parse_html(&body);
    bookmark.url = canonical;
    if bookmark.site.is_none() && !site.is_empty() {
        bookmark.site = Some(site);
    }
    if bookmark.title.is_empty() {
        bookmark.title = bookmark.url.clone();
    }
    if bookmark.added.is_none() {
        bookmark.added = Some(metadata::today());
    }

    let image_url = raw_image.and_then(|raw| resolve_url(&parsed, &raw));
    Ok(FetchResult {
        bookmark,
        image_url,
        html: body,
    })
}

pub fn extract_article(html: &str, url: &str) -> Option<String> {
    use dom_smoothie::{Config, Readability};
    let parsed_url = Url::parse(url).ok()?;
    let mut readability =
        Readability::new(html, Some(parsed_url.as_str()), Some(Config::default())).ok()?;
    let article = readability.parse().ok()?;
    let text = article.text_content.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

pub fn parse_html(body: &str) -> (Bookmark, Option<String>) {
    let doc = Html::parse_document(body);

    let title = meta_content(&doc, "og:title")
        .or_else(|| meta_content(&doc, "twitter:title"))
        .or_else(|| meta_name_content(&doc, "title"))
        .or_else(|| {
            let sel = Selector::parse("title").ok()?;
            doc.select(&sel)
                .next()
                .map(|n| collapse_ws(&n.text().collect::<String>()))
        })
        .unwrap_or_default();

    let description = meta_content(&doc, "og:description")
        .or_else(|| meta_content(&doc, "twitter:description"))
        .or_else(|| meta_name_content(&doc, "description"));

    let site_name = meta_content(&doc, "og:site_name");

    let author_strs = [
        meta_name_content(&doc, "author"),
        meta_content(&doc, "article:author"),
        meta_content(&doc, "book:author"),
    ];
    let mut authors: Vec<String> = author_strs
        .into_iter()
        .flatten()
        .flat_map(|s| split_authors(&s))
        .collect();
    authors.dedup();

    let year = meta_content(&doc, "article:published_time")
        .or_else(|| meta_name_content(&doc, "date"))
        .or_else(|| meta_name_content(&doc, "pubdate"))
        .or_else(|| meta_content(&doc, "og:updated_time"))
        .and_then(|s| extract_year(&s));

    let image = meta_content(&doc, "og:image")
        .or_else(|| meta_content(&doc, "og:image:url"))
        .or_else(|| meta_content(&doc, "twitter:image"))
        .or_else(|| meta_name_content(&doc, "twitter:image"));

    let bookmark = Bookmark {
        url: String::new(),
        title: collapse_ws(&title),
        description: description.map(|s| collapse_ws(&s)),
        authors,
        site: site_name.map(|s| collapse_ws(&s)),
        year,
        tags: vec![],
        added: None,
        files: vec![],
        preview: None,
    };
    (bookmark, image)
}

pub fn download_preview(image_url: &str) -> Result<(Vec<u8>, String)> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()?;

    let mut resp = client
        .get(image_url)
        .send()
        .with_context(|| format!("Failed to fetch image {}", image_url))?
        .error_for_status()?;

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_ascii_lowercase());

    let mut buf = Vec::new();
    let mut take = (&mut resp).take(MAX_IMAGE_BYTES + 1);
    take.read_to_end(&mut buf)
        .context("Failed to read image bytes")?;
    if buf.len() as u64 > MAX_IMAGE_BYTES {
        anyhow::bail!("Image exceeds {} bytes", MAX_IMAGE_BYTES);
    }

    let ext = ext_from_content_type(content_type.as_deref())
        .or_else(|| ext_from_url(image_url))
        .or_else(|| sniff_ext(&buf))
        .unwrap_or("jpg");

    if let Some((resized, new_ext)) = resize_to_jpeg(&buf) {
        return Ok((resized, new_ext.to_string()));
    }

    Ok((buf, ext.to_string()))
}

fn resize_to_jpeg(bytes: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let img = image::load_from_memory(bytes).ok()?;
    if img.width() <= PREVIEW_MAX_WIDTH {
        return None;
    }
    let new_height =
        (img.height() as u64 * PREVIEW_MAX_WIDTH as u64 / img.width() as u64).max(1) as u32;
    let resized = img.resize(
        PREVIEW_MAX_WIDTH,
        new_height,
        image::imageops::FilterType::Lanczos3,
    );
    let rgb = resized.to_rgb8();
    let mut out = Vec::new();
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, PREVIEW_JPEG_QUALITY);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some((out, "jpg"))
}

fn resolve_url(base: &Url, raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    Url::parse(raw)
        .or_else(|_| base.join(raw))
        .ok()
        .map(|u| u.to_string())
}

fn ext_from_content_type(ct: Option<&str>) -> Option<&'static str> {
    let ct = ct?.split(';').next()?.trim();
    match ct {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/svg+xml" => Some("svg"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

fn ext_from_url(url: &str) -> Option<&'static str> {
    let path = Url::parse(url).ok()?.path().to_ascii_lowercase();
    let ext = path.rsplit('.').next()?;
    match ext {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "avif" => Some("avif"),
        _ => None,
    }
}

fn sniff_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff" {
        return Some("jpg");
    }
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("png");
    }
    if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    None
}

fn meta_content(doc: &Html, property: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[property=\"{}\"]", property)).ok()?;
    doc.select(&selector)
        .find_map(|n| n.value().attr("content").map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
}

fn meta_name_content(doc: &Html, name: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[name=\"{}\"]", name)).ok()?;
    doc.select(&selector)
        .find_map(|n| n.value().attr("content").map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_authors(raw: &str) -> Vec<String> {
    let separators = [';', ','];
    let mut parts: Vec<String> = if raw.contains(';') {
        raw.split(';').map(|s| s.trim().to_string()).collect()
    } else if raw.contains(" and ") {
        raw.split(" and ").map(|s| s.trim().to_string()).collect()
    } else if raw.matches(',').count() >= 2 {
        raw.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        vec![raw.trim().to_string()]
    };
    parts.retain(|s| !s.is_empty());
    let _ = separators;
    parts
}

fn extract_year(s: &str) -> Option<u16> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) {
            if let Ok(year) = std::str::from_utf8(&bytes[i..i + 4])
                .unwrap_or("")
                .parse::<u16>()
                && (1900..=2100).contains(&year)
            {
                return Some(year);
            }
        }
        i += 1;
    }
    None
}
