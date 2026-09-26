//! Shortest-unique-prefix handling for finding ids and evidence digests.
//!
//! Full ids are `sha256:<64 hex>` and unwieldy to type or paste back into a
//! command. Every place that displays one instead shows the shortest hex
//! prefix (minimum [`MIN_PREFIX_HEX`] characters, `sha256:` dropped) that is
//! unique across a given run's ids; every place that accepts one accepts
//! that same prefix back, resolved against the same kind of candidate set.

use anyhow::{bail, Result};
use std::collections::BTreeSet;

/// The shortest prefix ever shown or accepted, regardless of how few ids are
/// in play.
pub const MIN_PREFIX_HEX: usize = 8;

/// The hex body of a full id, with any `sha256:` marker removed. Used both
/// for ids that already carry the marker and for bare hex prefixes.
pub fn hex_body(id: &str) -> &str {
    id.strip_prefix("sha256:").unwrap_or(id)
}

/// The shortest prefix length (>= [`MIN_PREFIX_HEX`]) that is unique across
/// every id in `ids`. Deterministic: depends only on the set of ids, not the
/// order they were discovered in.
pub fn prefix_len(ids: &BTreeSet<String>) -> usize {
    let mut len = MIN_PREFIX_HEX;
    while len < 64 {
        let mut seen = BTreeSet::new();
        let mut collided = false;
        for id in ids {
            let hex = hex_body(id);
            if !seen.insert(&hex[..len.min(hex.len())]) {
                collided = true;
                break;
            }
        }
        if !collided {
            return len;
        }
        len += 1;
    }
    64
}

/// The display form of `id`: its hex body truncated to `len` characters, no
/// `sha256:` marker.
pub fn display(id: &str, len: usize) -> String {
    let hex = hex_body(id);
    hex[..len.min(hex.len())].to_owned()
}

fn is_full_digest(input: &str) -> bool {
    input.len() == 71
        && input.starts_with("sha256:")
        && input[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Resolve a user-supplied reference against `candidates` (a set of full
/// `sha256:<64 hex>` ids). A full, well-formed digest is returned unchanged
/// so existing exact-id behavior (including "not found" errors from callers
/// that check membership themselves) is untouched. Anything else must be a
/// hex prefix of at least [`MIN_PREFIX_HEX`] characters (with or without a
/// leading `sha256:`); it is matched against `candidates`, erroring clearly
/// on zero or multiple matches. `what` names the kind of id, for messages
/// ("finding id", "evidence digest").
pub fn resolve(candidates: &BTreeSet<String>, input: &str, what: &str) -> Result<String> {
    if is_full_digest(input) {
        return Ok(input.to_owned());
    }
    let hex = input
        .strip_prefix("sha256:")
        .unwrap_or(input)
        .to_ascii_lowercase();
    if hex.len() < MIN_PREFIX_HEX || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(
            "{what} {input:?} is not a full sha256 id or a hex prefix of at least {MIN_PREFIX_HEX} characters"
        );
    }
    let matches = candidates
        .iter()
        .filter(|candidate| hex_body(candidate).starts_with(hex.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => bail!("no {what} matches prefix {input:?}"),
        [only] => Ok(only.clone()),
        many => bail!(
            "ambiguous {what} prefix {input:?}; candidates: {}",
            many.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> BTreeSet<String> {
        values
            .iter()
            .map(|v| format!("sha256:{}", v.repeat(64)))
            .collect()
    }

    #[test]
    fn prefix_len_stays_at_minimum_when_ids_diverge_early() {
        let set = ids(&["a", "b", "c"]);
        assert_eq!(prefix_len(&set), MIN_PREFIX_HEX);
    }

    #[test]
    fn prefix_len_grows_to_disambiguate_a_shared_prefix() {
        let mut set = BTreeSet::new();
        set.insert(format!("sha256:{}0{}", "a".repeat(8), "1".repeat(55)));
        set.insert(format!("sha256:{}1{}", "a".repeat(8), "2".repeat(55)));
        // Both share an 8-hex-char prefix ("aaaaaaaa"); the 9th char differs.
        assert_eq!(prefix_len(&set), 9);
    }

    #[test]
    fn resolve_passes_a_full_digest_through_unchanged_even_if_unknown() {
        let set = ids(&["a"]);
        let unknown = format!("sha256:{}", "f".repeat(64));
        assert_eq!(resolve(&set, &unknown, "finding id").unwrap(), unknown);
    }

    #[test]
    fn resolve_matches_a_bare_and_prefixed_hex_prefix() {
        let set = ids(&["a", "b"]);
        let full_a = format!("sha256:{}", "a".repeat(64));
        assert_eq!(resolve(&set, &"a".repeat(8), "finding id").unwrap(), full_a);
        assert_eq!(
            resolve(&set, &format!("sha256:{}", "a".repeat(8)), "finding id").unwrap(),
            full_a
        );
    }

    #[test]
    fn resolve_rejects_short_ambiguous_and_unmatched_prefixes() {
        let set = ids(&["a", "b"]);
        assert!(resolve(&set, "a1234", "finding id").is_err());
        assert!(resolve(&set, &"f".repeat(8), "finding id").is_err());
        let mut collide = BTreeSet::new();
        collide.insert(format!("sha256:{}", "a".repeat(64)));
        collide.insert(format!("sha256:aaaaaaaab{}", "1".repeat(55)));
        let error = resolve(&collide, &"a".repeat(8), "finding id").unwrap_err();
        assert!(error.to_string().contains("ambiguous"));
    }
}
