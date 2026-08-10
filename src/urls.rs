use anyhow::{Context, Result, bail};
use url::Url;

pub fn normalize(input: &str) -> Result<String> {
    let parsed = Url::parse(input.trim()).with_context(|| format!("Invalid URL: {}", input))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        bail!("URL must use http or https and include a host");
    }
    Ok(parsed.to_string())
}

pub fn canonical_key(input: &str) -> Option<String> {
    let parsed = Url::parse(input.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let host = parsed.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let mut key = host.to_string();
    if let Some(port) = parsed.port() {
        key.push_str(&format!(":{}", port));
    }

    let path = parsed.path().trim_end_matches('/');
    if !path.is_empty() {
        key.push_str(path);
    }

    let mut query: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(name, _)| !is_tracking_parameter(name))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    query.sort();

    if !query.is_empty() {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish();
        key.push('?');
        key.push_str(&encoded);
    }

    Some(key)
}

fn is_tracking_parameter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("utm_")
        || matches!(
            name.as_str(),
            "fbclid" | "gclid" | "dclid" | "mc_cid" | "mc_eid"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_ignores_common_url_variants() {
        let left = canonical_key("http://www.Example.com/article/?b=2&a=1&utm_source=test#part");
        let right = canonical_key("https://example.com/article?a=1&b=2");
        assert_eq!(left, right);
    }

    #[test]
    fn normalize_rejects_non_web_urls() {
        assert!(normalize("file:///tmp/bookmark").is_err());
        assert!(normalize("not a url").is_err());
    }
}
