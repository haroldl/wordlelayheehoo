//! This project will build a decision tree to try to fully solve Wordle.
//!
//! Each word stores five lowercase ASCII letters in a fixed-size byte array.
//! A Word occupies five bytes, including in arrays and vectors, compared with
//! eight bytes for a packed u64. No heap allocation or bit shifting is needed.
//! Normalizing case gives words consistent equality, hashing, and alphabetical
//! ordering. The private field keeps validation behind the constructor.

/// Five lowercase ASCII letters, stored inline without padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Word([u8; 5]);

impl Word {
    /// Creates a word from exactly five ASCII letters, normalizing case.
    /// Returns None for any other length or characters outside A-Z / a-z.
    /// This validates the representation, not membership in a Wordle dictionary.
    pub fn new(word: &str) -> Option<Self> {
        let mut bytes: [u8; 5] = word.as_bytes().try_into().ok()?;
        if !bytes.iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        bytes.make_ascii_lowercase();
        Some(Self(bytes))
    }

    /// Borrows the lowercase word as a string without allocating.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("Word contains only ASCII letters")
    }

    /// Borrows the letters in word order for direct access by index.
    pub fn as_bytes(&self) -> &[u8; 5] {
        &self.0
    }
}

fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests;
