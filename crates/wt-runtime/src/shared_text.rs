//! Immutable text with cheap clones and a content digest shared by every clone.
//!
//! A view retains its source allocation. Each view has its own lazy digest, so
//! source bytes and cached match/capture bytes are hashed at most once, even
//! when many independent rule invocations consume them. Replacing a source
//! creates a new value; an address or path is never used as content identity.
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::ops::{Deref, Range};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug)]
pub struct SharedText(Arc<TextStorage>);

#[derive(Debug)]
struct TextStorage {
    source: Arc<String>,
    range: Range<usize>,
    digest: OnceLock<[u8; 32]>,
}

impl SharedText {
    /// Return a view into validated UTF-8 boundaries without copying its bytes.
    pub(crate) fn slice(&self, range: Range<usize>) -> Self {
        assert!(self.get(range.clone()).is_some(), "invalid UTF-8 text view");
        if range == (0..self.len()) {
            return self.clone();
        }
        Self(Arc::new(TextStorage {
            source: Arc::clone(&self.0.source),
            range: self.0.range.start + range.start..self.0.range.start + range.end,
            digest: OnceLock::new(),
        }))
    }

    /// Count physical hashing only on initialization, not on cache hits.
    pub(crate) fn digest(&self, bytes_hashed: &mut u64) -> [u8; 32] {
        *self.0.digest.get_or_init(|| {
            *bytes_hashed = bytes_hashed.saturating_add(self.len() as u64);
            Sha256::digest(self.as_bytes()).into()
        })
    }

    #[cfg(test)]
    pub(crate) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0.source, &other.0.source)
    }
}

impl From<String> for SharedText {
    fn from(source: String) -> Self {
        let len = source.len();
        Self(Arc::new(TextStorage {
            source: Arc::new(source),
            range: 0..len,
            digest: OnceLock::new(),
        }))
    }
}

impl From<&str> for SharedText {
    fn from(source: &str) -> Self {
        source.to_owned().into()
    }
}

impl Deref for SharedText {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0.source[self.0.range.clone()]
    }
}

impl AsRef<str> for SharedText {
    fn as_ref(&self) -> &str {
        self
    }
}

impl AsRef<[u8]> for SharedText {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<T: AsRef<str> + ?Sized> PartialEq<T> for SharedText {
    fn eq(&self, other: &T) -> bool {
        **self == *other.as_ref()
    }
}
impl Eq for SharedText {}

impl fmt::Display for SharedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self)
    }
}

impl Serialize for SharedText {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_and_views_share_bytes_but_hash_the_correct_content() {
        let text = SharedText::from("aé\r\nz");
        let mut hashed = 0;
        let expected: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        assert_eq!(text.digest(&mut hashed), expected);
        assert_eq!(text.clone().digest(&mut hashed), expected);
        assert_eq!(hashed, text.len() as u64);
        let view = text.slice(1..3);
        assert_eq!(view, "é");
        assert!(view.shares_storage(&text));
        let expected: [u8; 32] = Sha256::digest("é".as_bytes()).into();
        assert_eq!(view.digest(&mut hashed), expected);
        assert_eq!(view.clone().digest(&mut hashed), expected);
        assert_eq!(hashed, (text.len() + view.len()) as u64);
    }

    #[test]
    fn replacing_same_length_text_never_inherits_a_digest() {
        let mut text = SharedText::from("bad");
        let old = text.clone();
        let mut hashed = 0;
        let before = text.digest(&mut hashed);
        text = "new".into();
        assert_ne!(text.digest(&mut hashed), before);
        assert_eq!(old.digest(&mut hashed), before);
        assert_eq!(hashed, 6);
    }
}
