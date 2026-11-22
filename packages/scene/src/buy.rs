//! A `buy` entry's seller, an https URL and the day that URL was checked.

use std::collections::BTreeSet;

use crate::diagnostic::{Code, Findings};
use crate::syntax::Node;

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
        if let Some(url) = item.get("url") {
            parse_url(url, out);
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

fn parse_url(node: &Node, out: &mut Findings) {
    let Some(url) = node.str() else {
        return;
    };
    if let Err(fault) = split_url(url) {
        let (message, hint) = fault.text();
        out.push(Code::Syntax, node.span, message, hint.into());
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
/// fragment.
fn split_url(url: &str) -> Result<(), UrlFault> {
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
    Ok(())
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
