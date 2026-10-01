//! Immutable Wordle game state and searches over possible outcomes.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use crate::{DEFAULT_SOLVER_WORKERS, MAX_GUESSES, solver};
use crate::words::{Result, WORDS, Word};

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
            .copied()
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
            .any(|(_, result)| *result == Result::ALL_GREEN)
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

    /// Returns the worst-case additional guesses for the first winning strategy found.
    /// Uses the full remaining budget; does not minimize the number of guesses.
    /// Returns Some(0) for a consistent solved state, or None if no dictionary
    /// target fits or no strategy guarantees a win within MAX_GUESSES total.
    /// Searches all unguessed dictionary words, including non-candidate probes.
    /// Exact search can be expensive for large candidate sets.
    pub fn minimax_guesses(&self) -> Option<usize> {
        self.minimax_guesses_with_workers(DEFAULT_SOLVER_WORKERS)
    }

    /// Runs minimax using a fixed pool of workers; workers must be positive.
    pub fn minimax_guesses_with_workers(&self, workers: usize) -> Option<usize> {
        self.minimax_guesses_using(&mut solver::MinimaxSolver::new(workers))
    }

    fn minimax_guesses_using(&self, solver: &mut solver::MinimaxSolver) -> Option<usize> {
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
        solver.strategy_guesses(
            &allowed,
            &candidates,
            MAX_GUESSES.saturating_sub(self.guesses.len()),
        )
    }

    /// Builds an adaptive strategy that wins within MAX_GUESSES total guesses.
    /// Accepts the first proven strategy without searching for a shorter one.
    /// Supports an empty state to choose the opening word as part of the search.
    pub fn decision_tree(&self) -> Option<solver::DecisionTree> {
        self.decision_tree_with_workers(DEFAULT_SOLVER_WORKERS)
    }

    /// Builds a strategy with the configured worker count. Returns None for
    /// inconsistent observations or no strategy within the remaining guess budget.
    pub fn decision_tree_with_workers(&self, workers: usize) -> Option<solver::DecisionTree> {
        let candidates: Vec<_> = self.possible_solutions().collect();
        if candidates.is_empty() {
            return None;
        }
        if self.is_solved() {
            return Some(solver::DecisionTree::Solved);
        }
        let allowed: Vec<_> = WORDS.iter().copied()
            .filter(|&word| !self.has_guessed(word)).collect();
        solver::MinimaxSolver::new(workers).decision_tree(
            &allowed, &candidates, MAX_GUESSES.saturating_sub(self.guesses.len()),
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
#[cfg(test)]
pub(crate) fn max_minimax_guesses(game_states: &HashSet<GameState>, workers: usize) -> Option<usize> {
    let mut solver = solver::MinimaxSolver::new(workers);
    game_states.iter()
        .map(|state| state.minimax_guesses_using(&mut solver))
        .try_fold(0usize, |worst, next| next.map(|count| worst.max(count)))
}

