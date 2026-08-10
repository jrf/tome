use anyhow::{Context, Result};
use scraper::{Html, Selector};
use std::time::Duration;
use url::Url;

use crate::metadata;
use crate::model::Bookmark;

const USER_AGENT: &str = "Mozilla/5.0 (compatible; cairn/0.1; +https://github.com/jrf/cairn)";

pub struct FetchResult {
    pub bookmark: Bookmark,
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

    let mut bookmark = parse_html(&body);
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

    Ok(FetchResult {
        bookmark,
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

pub fn parse_html(body: &str) -> Bookmark {
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

    Bookmark {
        url: String::new(),
        title: collapse_ws(&title),
        description: description.map(|s| collapse_ws(&s)),
        authors,
        site: site_name.map(|s| collapse_ws(&s)),
        year,
        tags: vec![],
        added: None,
        files: vec![],
    }
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
