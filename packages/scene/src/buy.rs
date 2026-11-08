//! A `buy` entry's seller and URL. Allowed sellers are `buy-sellers.jsonc`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use crate::diagnostic::{Code, Findings};
use crate::source::{Sources, Span};
use crate::syntax::Node;

const SELLERS_FILE: &str = "packages/scene/buy-sellers.jsonc";
const SELLER_HINT: &str = "sellers are listed in `packages/scene/buy-sellers.jsonc`. a new seller is a one-line addition, reviewed for affiliate and tracking links";

/// Seller, an https URL and the day that URL was checked. `shape` reports a wrong type or an
/// unknown key. An empty list is refused: leave `buy` out instead.
pub(crate) fn check(node: &Node, out: &mut Findings) {
    let Some(items) = node.items() else {
        return;
    };
    if items.is_empty() {
        out.push(
            Code::Syntax,
            node.span,
            "`buy` is empty",
            "leave `buy` out when the part has no link".into(),
        );
        return;
    }
    let mut seen = BTreeSet::new();
    for item in items {
        if item.props().is_none() {
            continue;
        }
        for key in ["seller", "url", "checked"] {
            if item.get(key).is_none() {
                out.push(
                    Code::Syntax,
                    item.span,
                    format!("a buy entry has no `{key}`"),
                    "each entry needs `seller`, `url` and `checked`".into(),
                );
            }
        }
        let seller = item.get("seller").and_then(Node::str);
        if let Some(seller_node) = item.get("seller")
            && seller.is_some_and(|text| text.trim().is_empty())
        {
            out.push(
                Code::Syntax,
                seller_node.span,
                "a buy entry's `seller` is empty",
                "name the seller".into(),
            );
        }
        let host = item.get("url").and_then(|url| parse_url(url, out));
        if let Some(seller_node) = item.get("seller")
            && let Some(name) = seller.filter(|name| !name.trim().is_empty())
        {
            match_seller(seller_node.span, name, host, item.get("url"), out);
        }
        if let Some(checked) = item.get("checked")
            && checked.str().is_some_and(|text| !calendar_date(text))
        {
            out.push(
                Code::Syntax,
                checked.span,
                "a buy entry's `checked` is not a calendar date",
                "write the day the link was checked, as \"2026-11-08\"".into(),
            );
        }
        if let (Some(name), Some(url), Some(checked)) = (
            seller,
            item.get("url").and_then(Node::str),
            item.get("checked").and_then(Node::str),
        ) && !seen.insert((name.to_string(), url.to_string(), checked.to_string()))
        {
            out.push(
                Code::Syntax,
                item.span,
                "`buy` repeats an entry",
                "leave one of the two entries".into(),
            );
        }
    }
}

fn match_seller(
    seller_span: Span,
    seller: &str,
    host: Option<&str>,
    url: Option<&Node>,
    out: &mut Findings,
) {
    let Some(allowed) = sellers().get(seller) else {
        out.push(
            Code::Syntax,
            seller_span,
            "`seller` is not an allowed seller",
            SELLER_HINT.into(),
        );
        return;
    };
    let (Some(host), Some(url)) = (host, url) else {
        return;
    };
    if allowed.iter().any(|one| one == host) {
        return;
    }
    out.push(
        Code::Syntax,
        url.span,
        "`url` host is not this seller's",
        SELLER_HINT.into(),
    );
}

fn parse_url<'a>(node: &'a Node, out: &mut Findings) -> Option<&'a str> {
    let url = node.str()?;
    match split_url(url) {
        Ok(host) => Some(host),
        Err(fault) => {
            let (message, hint) = fault.text();
            out.push(Code::Syntax, node.span, message, hint.into());
            None
        }
    }
}

enum UrlFault {
    Scheme,
    Whitespace,
    Userinfo,
    Query,
    Fragment,
    NoHost,
}

impl UrlFault {
    fn text(self) -> (&'static str, &'static str) {
        match self {
            UrlFault::Scheme => (
                "`url` is not an https URL",
                "write a lowercase https:// URL",
            ),
            UrlFault::Whitespace => (
                "`url` has whitespace or a control character",
                "remove the whitespace or control character",
            ),
            UrlFault::Userinfo => (
                "`url` has userinfo",
                "write the host with no `@`. an `@` before the host opens a different site",
            ),
            UrlFault::Query => ("`url` has a query string", "remove the query string"),
            UrlFault::Fragment => ("`url` has a fragment", "remove the fragment"),
            UrlFault::NoHost => ("`url` has no host", "name the host"),
        }
    }
}

/// `https://` with no userinfo, no whitespace and no control character, no query and no
/// fragment. The host is the authority. The seller table decides whether that host is allowed.
fn split_url(url: &str) -> Result<&str, UrlFault> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(UrlFault::Whitespace);
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(UrlFault::Scheme);
    };
    if rest.contains('?') {
        return Err(UrlFault::Query);
    }
    if rest.contains('#') {
        return Err(UrlFault::Fragment);
    }
    let authority = match rest.split_once('/') {
        Some((authority, _)) => authority,
        None => rest,
    };
    if authority.contains('@') {
        return Err(UrlFault::Userinfo);
    }
    if authority.is_empty() {
        return Err(UrlFault::NoHost);
    }
    Ok(authority)
}

fn sellers() -> &'static BTreeMap<String, Vec<String>> {
    static SELLERS: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
    SELLERS.get_or_init(load_sellers)
}

fn load_sellers() -> BTreeMap<String, Vec<String>> {
    let text = include_str!("../buy-sellers.jsonc").to_string();
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new(SELLERS_FILE), text);
    let mut findings = Findings::default();
    let Some(root) = crate::syntax::parse(&sources, file, &mut findings) else {
        let diagnostics = findings.into_diagnostics(&sources);
        panic!(
            "{SELLERS_FILE} did not parse:\n{}",
            diagnostics
                .iter()
                .map(|finding| finding.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    };
    let Some(props) = root.props() else {
        panic!("{SELLERS_FILE} is an object of seller to hosts");
    };
    let mut sellers = BTreeMap::new();
    for prop in props {
        if prop.key.is_empty() {
            panic!("{SELLERS_FILE} has an empty seller");
        }
        let Some(hosts) = prop.value.items() else {
            panic!("{SELLERS_FILE}: {} is not a list of hosts", prop.key);
        };
        let mut list = Vec::new();
        for host_node in hosts {
            let Some(host) = host_node.str() else {
                panic!(
                    "{SELLERS_FILE}: {} has a host that is not a string",
                    prop.key
                );
            };
            if !dotted_ascii_host(host) {
                panic!(
                    "{SELLERS_FILE}: {} host {host} is not a dotted ASCII host",
                    prop.key
                );
            }
            list.push(host.to_string());
        }
        if list.is_empty() {
            panic!("{SELLERS_FILE}: {} lists no host", prop.key);
        }
        if sellers.insert(prop.key.clone(), list).is_some() {
            panic!("{SELLERS_FILE} lists {} twice", prop.key);
        }
    }
    if sellers.is_empty() {
        panic!("{SELLERS_FILE} lists no seller");
    }
    sellers
}

/// A host the seller table may name. A URL host is allowed only when it equals one of these,
/// so the table is where the dotted-ASCII rule is checked.
fn dotted_ascii_host(host: &str) -> bool {
    let bytes = host.as_bytes();
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'))
        && host.contains('.')
        && host.split('.').all(|label| !label.is_empty())
}

/// `YYYY-MM-DD` that is a real day. There is no clock here, so a later day is still valid.
fn calendar_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits = |slice: &[u8]| slice.iter().all(u8::is_ascii_digit);
    if !digits(&bytes[..4]) || !digits(&bytes[5..7]) || !digits(&bytes[8..]) {
        return false;
    }
    let Ok(year) = text[..4].parse::<u16>() else {
        return false;
    };
    let Ok(month) = text[5..7].parse::<u8>() else {
        return false;
    };
    let Ok(day) = text[8..].parse::<u8>() else {
        return false;
    };
    if year == 0 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=last).contains(&day)
}
