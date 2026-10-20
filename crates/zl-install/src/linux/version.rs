use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
    Newer,
    Same,
    Older,
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
enum Seg {
    Num(u64),
    Alpha(String),
}

fn is_unknown(v: &str) -> bool {
    let t = v.trim().to_ascii_lowercase();
    t.is_empty() || t == "unknown" || t == "0" || t == "latest" || t == "continuous"
}

fn segments(v: &str) -> Vec<Seg> {
    let v = v.trim();
    let v = v
        .split_once(':')
        .map(|(e, r)| {
            if e.chars().all(|c| c.is_ascii_digit()) {
                r
            } else {
                v
            }
        })
        .unwrap_or(v);
    let v = v
        .strip_prefix(['v', 'V'])
        .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(v);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_digit: Option<bool> = None;
    for c in v.chars() {
        let d = c.is_ascii_digit();
        if !c.is_ascii_alphanumeric() {
            flush(&mut out, &mut cur, cur_digit);
            cur_digit = None;
            continue;
        }
        if cur_digit.is_some_and(|x| x != d) {
            flush(&mut out, &mut cur, cur_digit);
        }
        cur.push(c.to_ascii_lowercase());
        cur_digit = Some(d);
    }
    flush(&mut out, &mut cur, cur_digit);
    out
}

fn flush(out: &mut Vec<Seg>, cur: &mut String, digit: Option<bool>) {
    if cur.is_empty() {
        return;
    }
    out.push(match digit {
        Some(true) => Seg::Num(cur.parse().unwrap_or(u64::MAX)),
        _ => Seg::Alpha(std::mem::take(cur)),
    });
    cur.clear();
}

fn alpha_rank(a: &str) -> i32 {
    match a {
        "alpha" | "a" => -4,
        "beta" | "b" => -3,
        "rc" | "pre" | "preview" => -2,
        "dev" | "nightly" | "snapshot" => -5,
        _ => 0,
    }
}

pub fn compare(a: &str, b: &str) -> Ordering {
    let (sa, sb) = (segments(a), segments(b));
    let n = sa.len().max(sb.len());
    for i in 0..n {
        let ord = match (sa.get(i), sb.get(i)) {
            (Some(Seg::Num(x)), Some(Seg::Num(y))) => x.cmp(y),
            (Some(Seg::Alpha(x)), Some(Seg::Alpha(y))) => {
                alpha_rank(x).cmp(&alpha_rank(y)).then_with(|| x.cmp(y))
            }

            (Some(Seg::Num(_)), Some(Seg::Alpha(_))) => Ordering::Greater,
            (Some(Seg::Alpha(_)), Some(Seg::Num(_))) => Ordering::Less,
            (Some(Seg::Num(x)), None) => x.cmp(&0),
            (None, Some(Seg::Num(y))) => 0.cmp(y),
            (Some(Seg::Alpha(x)), None) => {
                if alpha_rank(x) < 0 {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (None, Some(Seg::Alpha(y))) => {
                if alpha_rank(y) < 0 {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (None, None) => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

pub fn relation(candidate: &str, installed: &str) -> Relation {
    if is_unknown(candidate) || is_unknown(installed) {
        return Relation::Unknown;
    }
    match compare(candidate, installed) {
        Ordering::Greater => Relation::Newer,
        Ordering::Equal => Relation::Same,
        Ordering::Less => Relation::Older,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering() {
        assert_eq!(compare("0.7.0", "0.6.0"), Ordering::Greater);
        assert_eq!(compare("1.2.0", "1.10.0"), Ordering::Less);
        assert_eq!(compare("v1.2.0", "1.2.0"), Ordering::Equal);
        assert_eq!(compare("2.10-9.fc38", "2.10-3"), Ordering::Greater);
        assert_eq!(compare("1.0.0-beta", "1.0.0"), Ordering::Less);
        assert_eq!(compare("1.0.0-rc1", "1.0.0-beta2"), Ordering::Greater);
        assert_eq!(compare("1:0.5", "0.9"), Ordering::Less);
        assert_eq!(compare("0.6.0", "0.6"), Ordering::Equal);
    }

    #[test]
    fn relations() {
        assert_eq!(relation("0.7.0", "0.6.0"), Relation::Newer);
        assert_eq!(relation("0.6.0", "0.6.0"), Relation::Same);
        assert_eq!(relation("0.5.0", "0.6.0"), Relation::Older);
        assert_eq!(relation("unknown", "0.6.0"), Relation::Unknown);
    }
}
