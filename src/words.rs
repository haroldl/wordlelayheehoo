//! Word representation, feedback scoring, and the compiled dictionary.
//!
//! Each word stores five lowercase ASCII letters in a fixed-size byte array.
//! A Word occupies five bytes, including in arrays and vectors, compared with
//! eight bytes for a packed u64. No heap allocation or bit shifting is needed.
//! Normalizing case gives words consistent equality, hashing, and alphabetical
//! ordering. The private field keeps runtime validation behind the constructor.
//!
//! Cargo runs build.rs to filter WORD.LST to five ASCII letters per entry and
//! normalize case, preserving file order and duplicates. The generated WORDS
//! slice contains only the processed bytes; no runtime parsing or allocation is
//! needed. Cargo regenerates it when WORD.LST changes.

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

/// Feedback for one letter in a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LetterResult {
    /// No unmatched occurrence remains in the target, including excess repeats.
    Grey,
    /// The letter occurs in the target, but at a different position.
    Gold,
    /// The letter matches the target at this position.
    Green,
}

/// Feedback for the five letters of a guess, in positional order.
/// Stored inline as five bytes without heap allocation.
/// This game type is separate from Rust's std::result::Result<T, E>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Result([LetterResult; 5]);

impl Result {
    /// Creates feedback with exactly one result per letter.
    pub fn new(letters: [LetterResult; 5]) -> Self {
        Self(letters)
    }

    /// Scores a guess against a target using Wordle's duplicate-letter rules.
    /// Exact matches consume target letters first. Remaining guess letters are
    /// scored left to right, with each unmatched target occurrence used at most
    /// once. Both words are copied inline; scoring requires no heap allocation.
    pub fn from_guess(guess: Word, target: Word) -> Self {
        let mut letters = [LetterResult::Grey; 5];
        let mut remaining = [0u8; 26];

        for (position, result) in letters.iter_mut().enumerate() {
            if guess.0[position] == target.0[position] {
                *result = LetterResult::Green;
            } else {
                remaining[usize::from(target.0[position] - b'a')] += 1;
            }
        }

        for (position, result) in letters.iter_mut().enumerate() {
            if *result == LetterResult::Green {
                continue;
            }
            let count = &mut remaining[usize::from(guess.0[position] - b'a')];
            if *count > 0 {
                *result = LetterResult::Gold;
                *count -= 1;
            }
        }

        Self(letters)
    }

    /// Borrows the results in guess order for indexing or iteration.
    pub fn as_slice(&self) -> &[LetterResult] {
        &self.0
    }
}

// Before compiling this crate, Cargo runs build.rs, which reads WORD.LST,
// keeps lines containing exactly five ASCII letters, and lowercases them.
// It preserves file order and duplicates, then writes Rust source to
// OUT_DIR/words.rs defining a static slice with one entry per accepted line:
//     pub static WORDS: &[Word] = &[Word(*b"apple"), /* ... */];
// Here, *b"apple" supplies the [u8; 5] stored inside Word.
// env! and concat! resolve the generated file's path at compile time; include!
// inserts its declarations into this module, where Word's private field is
// accessible. The backing array is embedded in the executable, so WORDS needs
// no runtime file access, parsing, or heap allocation. build.rs tells Cargo to
// regenerate this file whenever WORD.LST changes.
include!(concat!(env!("OUT_DIR"), "/words.rs"));
