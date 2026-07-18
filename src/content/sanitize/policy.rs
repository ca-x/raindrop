use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use ammonia::{Builder, UrlRelative};
use url::Url;

const MAX_URL_BYTES: usize = 4_096;
const MAX_ALT_BYTES: usize = 4_096;
const MAX_DIMENSION: u32 = 16_384;

pub(super) fn sanitize_html(base_url: &str, input: &str, keep_image_source: bool) -> String {
    let base = base_url.to_owned();
    let mut builder = Builder::new();
    builder
        .tags(allowed_tags())
        .clean_content_tags(clean_content_tags())
        .generic_attributes(HashSet::new())
        .tag_attributes(tag_attributes(keep_image_source))
        .url_schemes(HashSet::from(["http", "https"]))
        .url_relative(UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer nofollow"))
        .attribute_filter(move |element, attribute, value| {
            filter_attribute(&base, element, attribute, value, keep_image_source)
        });
    builder.clean(input).to_string()
}

fn allowed_tags() -> HashSet<&'static str> {
    HashSet::from([
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "caption",
        "code",
        "col",
        "colgroup",
        "dd",
        "del",
        "details",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "kbd",
        "li",
        "mark",
        "ol",
        "p",
        "pre",
        "q",
        "rp",
        "rt",
        "ruby",
        "s",
        "samp",
        "small",
        "span",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "tr",
        "u",
        "ul",
        "var",
    ])
}

fn clean_content_tags() -> HashSet<&'static str> {
    HashSet::from([
        "base", "embed", "form", "frame", "frameset", "head", "iframe", "link", "math", "meta",
        "noscript", "object", "script", "style", "svg", "template",
    ])
}

fn tag_attributes(keep_image_source: bool) -> HashMap<&'static str, HashSet<&'static str>> {
    let mut attributes = HashMap::from([
        ("a", HashSet::from(["href"])),
        ("img", HashSet::from(["alt", "width", "height"])),
        ("td", HashSet::from(["colspan", "rowspan"])),
        ("th", HashSet::from(["colspan", "rowspan"])),
    ]);
    if keep_image_source {
        attributes
            .get_mut("img")
            .expect("img attribute set exists")
            .insert("src");
    }
    attributes
}

fn filter_attribute<'a>(
    base: &str,
    element: &str,
    attribute: &str,
    value: &'a str,
    keep_image_source: bool,
) -> Option<Cow<'a, str>> {
    match (element, attribute) {
        ("a", "href") => normalize_url(base, value).map(Cow::Owned),
        ("img", "src") if keep_image_source => normalize_url(base, value).map(Cow::Owned),
        ("img", "alt") if keep_image_source => Some(Cow::Borrowed(value)),
        ("img", "alt") if value.len() <= MAX_ALT_BYTES => Some(Cow::Borrowed(value)),
        ("img", "width" | "height") if keep_image_source => Some(Cow::Borrowed(value)),
        ("img", "width" | "height") => value
            .parse::<u32>()
            .ok()
            .filter(|dimension| (1..=MAX_DIMENSION).contains(dimension))
            .map(|dimension| Cow::Owned(dimension.to_string())),
        ("td" | "th", "colspan" | "rowspan") => value
            .parse::<u32>()
            .ok()
            .filter(|span| (1..=100).contains(span))
            .map(|span| Cow::Owned(span.to_string())),
        ("img", "src") => None,
        _ => Some(Cow::Borrowed(value)),
    }
}

fn normalize_url(base: &str, raw: &str) -> Option<String> {
    let base = Url::parse(base).ok()?;
    if !matches!(base.scheme(), "http" | "https")
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return None;
    }
    let mut url = base.join(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    let normalized = url.to_string();
    (normalized.len() <= MAX_URL_BYTES).then_some(normalized)
}
