//! This project will build a decision tree to try to fully solve Wordle.
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

mod solver;

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use clap::Parser;
use std::num::NonZeroUsize;

/// Maximum number of guesses allowed in a game.
pub const MAX_GUESSES: usize = 6;

/// Default number of minimax workers; override with --workers N.
pub const DEFAULT_SOLVER_WORKERS: usize = 10;

#[derive(Parser)]
#[command(version, about = "Wordle minimax solver")]
struct Args {
    /// Number of solver worker threads
    #[arg(
        short = 'w',
        long,
        default_value_t = NonZeroUsize::new(DEFAULT_SOLVER_WORKERS)
            .expect("default worker count must be positive")
    )]
    workers: NonZeroUsize,
}

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

/// An immutable, unordered set of guesses and their observed feedback.
/// Adding observations returns a new state, leaving the original unchanged.
/// Each word can be guessed only once, regardless of its feedback.
/// Equality compares the pairs regardless of insertion order. This stores
/// observations only; it does not check whether they share a possible target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameState {
    guesses: HashSet<(Word, Result)>,
}

// Hash a canonical ordering so equal states have equal hashes even when their
// guesses were inserted in different orders. This lets states form a HashSet.
impl Hash for GameState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut pairs: Vec<_> = self
            .guesses
            .iter()
            .map(|&(word, result)| (word, result.0.map(|letter| letter as u8)))
            .collect();
        pairs.sort_unstable();
        pairs.hash(state);
    }
}

impl GameState {
    /// Creates a game state with no guesses.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a new state containing this guess and its observed feedback.
    /// Returns None if this word has already been guessed, regardless of feedback.
    /// The original state is unchanged.
    /// Feedback is recorded as supplied, without checking target consistency.
    #[must_use = "with_guess returns a new state without changing the original"]
    pub fn with_guess(&self, word: Word, result: Result) -> Option<Self> {
        if self.has_guessed(word) {
            return None;
        }
        let mut guesses = self.guesses.clone();
        guesses.insert((word, result));
        Some(Self { guesses })
    }

    /// Returns whether this word has already been guessed.
    pub fn has_guessed(&self, word: Word) -> bool {
        self.guesses.iter().any(|&(guess, _)| guess == word)
    }

    /// Returns the distinct next states for a guess against current candidates.
    /// Each possible target supplies feedback; targets with identical feedback
    /// produce one state. Existing observations are preserved and self is unchanged.
    /// Returns an empty set if the word was already guessed or no target matches.
    /// Like possible_states_after_guesses, this also scores guesses after a win.
    pub fn make_guess(&self, guess: Word) -> HashSet<Self> {
        if self.has_guessed(guess) {
            return HashSet::new();
        }
        let results: HashSet<Result> = self
            .possible_solutions()
            .map(|target| Result::from_guess(guess, target))
            .collect();

        results
            .into_iter()
            .filter_map(|result| self.with_guess(guess, result))
            .collect()
    }

    /// Returns true if any recorded guess received five green results.
    /// Having only one possible solution left does not count as finding it;
    /// the target must have been guessed correctly.
    pub fn is_solved(&self) -> bool {
        self.guesses
            .iter()
            .any(|(_, result)| result.0 == [LetterResult::Green; 5])
    }

    /// Returns true when the guess limit has been reached without a correct guess.
    /// Each recorded pair represents one distinct guessed word.
    pub fn is_game_lost(&self) -> bool {
        self.guesses.len() >= MAX_GUESSES && !self.is_solved()
    }

    /// Lazily yields words from WORDS consistent with every recorded result.
    /// Each candidate is treated as the target and all guesses are scored again,
    /// including duplicate-letter rules. An empty state allows every word;
    /// contradictory observations yield no solutions.
    ///
    /// Candidates are copied in WORDS order, preserving any dictionary duplicates.
    /// The iterator borrows this state and allocates no candidate collection.
    pub fn possible_solutions(&self) -> impl Iterator<Item = Word> + '_ {
        WORDS.iter().copied().filter(|&candidate| {
            self.guesses
                .iter()
                .all(|&(guess, result)| Result::from_guess(guess, candidate) == result)
        })
    }

    /// Returns the minimum worst-case number of additional guesses to win.
    /// Returns Some(0) for a consistent solved state, or None if no dictionary
    /// target fits or no strategy guarantees a win within MAX_GUESSES total.
    /// Searches all unguessed dictionary words, including non-candidate probes.
    /// Exact search can be expensive for large candidate sets.
    pub fn minimax_guesses(&self) -> Option<usize> {
        self.minimax_guesses_with_workers(DEFAULT_SOLVER_WORKERS)
    }

    /// Runs minimax using a fixed pool of workers; workers must be positive.
    pub fn minimax_guesses_with_workers(&self, workers: usize) -> Option<usize> {
        self.minimax_guesses_using(&solver::MinimaxSolver::new(workers))
    }

    fn minimax_guesses_using(&self, solver: &solver::MinimaxSolver) -> Option<usize> {
        let candidates: Vec<_> = self.possible_solutions().collect();
        if candidates.is_empty() {
            return None;
        }
        if self.is_solved() {
            return Some(0);
        }
        let allowed: Vec<_> = WORDS.iter().copied()
            .filter(|&word| !self.has_guessed(word))
            .collect();
        solver.minimum_guesses(
            &allowed,
            &candidates,
            MAX_GUESSES.saturating_sub(self.guesses.len()),
        )
    }

    /// Borrows the recorded pairs. Iteration order is unspecified.
    pub fn guesses(&self) -> &HashSet<(Word, Result)> {
        &self.guesses
    }
}

/// Builds every distinct state attainable for these guesses against WORDS.
/// Each target supplies all feedback in a state; impossible combinations of
/// results are never generated. All supplied guesses are scored, even if an
/// earlier guess matches the target. Equal states are stored only once.
/// Returns an empty set when the supplied guesses contain a repeated word.
pub fn possible_states_after_guesses(guesses: &[Word]) -> HashSet<GameState> {
    if guesses.iter().collect::<HashSet<_>>().len() != guesses.len() {
        return HashSet::new();
    }
    WORDS
        .iter()
        .filter_map(|&target| {
            guesses.iter().try_fold(GameState::new(), |state, &guess| {
                state.with_guess(guess, Result::from_guess(guess, target))
            })
        })
        .collect()
}

/// Reuses one pool across states; queued work is inside each minimax search.
fn max_minimax_guesses(game_states: &HashSet<GameState>, workers: usize) -> Option<usize> {
    let solver = solver::MinimaxSolver::new(workers);
    game_states.iter()
        .map(|state| state.minimax_guesses_using(&solver))
        .try_fold(0usize, |worst, next| next.map(|count| worst.max(count)))
}

fn main() {
    let args = Args::parse();
    let workers = args.workers.get();
    println!("Using {workers} minimax workers.");

    println!("Loaded {} five-letter words.", WORDS.len());
    // Fixed opening for faster development; the model also supports an empty state.
    let first_guess = Word(*b"arose");
    // let second_guess = Word(*b"unlit");
    // let game_states = possible_states_after_guesses(&[first_guess, second_guess]);
    // println!(
    //     "{} possible game states after guessing {} and {}.",
    //     game_states.len(),
    //     first_guess.as_str(),
    //     second_guess.as_str()
    // );
    let game_states = possible_states_after_guesses(&[first_guess]);
    println!(
        "{} possible game states after guessing only {}.",
        game_states.len(),
        first_guess.as_str()
    );

    let max_guesses = max_minimax_guesses(&game_states, workers);

    match max_guesses {
        Some(count) => println!("Worst case: {count} additional guesses after the opening."),
        None => println!(
            "At least one state cannot be guaranteed solved within {MAX_GUESSES} total guesses."
        ),
    }
}

#[cfg(test)]
mod tests;
