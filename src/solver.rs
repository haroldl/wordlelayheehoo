//! Bounded strategy search with a reusable, fixed-size worker pool.
//! Using the full remaining budget, queued jobs evaluate proposed first guesses.
//! The first proven winning strategy is accepted without iterative deepening.
//! Each job searches its feedback subtrees recursively; workers share proven
//! subproblem results and cached guess–target feedback. The feedback table
//! persists across searches in the same pool; each allocated pair uses one byte. Workers never wait for jobs in their own pool, avoiding
//! nested-pool deadlocks and limiting total concurrency to the configured count.
//! Previously guessed words are excluded on entry. Deeper repeated guesses give
//! one non-winning group with every candidate and are rejected as no progress.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use crate::words::{Result, Word};

const GUESSES_PER_JOB: usize = 16;

/// Position in one Search's sorted, deduplicated list of allowed words.
/// This is not an index into the global WORDS dictionary, nor is it portable
/// between searches with different allowed words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
struct WordIndex(usize);

/// Position in a FeedbackTable's sorted vocabulary, which may include words
/// from earlier searches. Search::feedback_indices maps WordIndex to this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct FeedbackWordIndex(usize);

/// Dense, precomputed feedback table in a stable sorted word order.
/// Each pair occupies one byte. Workers share the completed table read-only.
struct FeedbackTable {
    words: Vec<Word>,
    patterns: Vec<Result>,
}

impl FeedbackTable {
    fn new(words: Vec<Word>) -> Self {
        let cells = words.len().checked_mul(words.len()).expect("feedback table too large");
        let mut patterns = Vec::with_capacity(cells);
        for &guess in &words {
            for &target in &words {
                patterns.push(Result::from_guess(guess, target));
            }
        }
        Self { words, patterns }
    }

    fn pattern(&self, guess: FeedbackWordIndex, target: FeedbackWordIndex) -> Result {
        self.patterns[guess.0 * self.words.len() + target.0]
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A strategy whose branches contain only reachable feedback outcomes.
/// Solved is reached only after the target has actually been guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionTree {
    Solved,
    Guess {
        word: Word,
        branches: HashMap<Result, DecisionTree>,
    },
}

impl DecisionTree {
    /// The next guess, or None when already solved.
    pub fn guess(&self) -> Option<Word> {
        match self {
            Self::Solved => None,
            Self::Guess { word, .. } => Some(*word),
        }
    }

    /// Follows observed feedback; None means it is not reachable in this tree.
    pub fn after_result(&self, result: Result) -> Option<&Self> {
        match self {
            Self::Solved => None,
            Self::Guess { branches, .. } => branches.get(&result),
        }
    }

    /// Maximum number of additional guesses along any branch.
    pub fn worst_case_guesses(&self) -> usize {
        match self {
            Self::Solved => 0,
            Self::Guess { branches, .. } => 1 + branches.values()
                .map(Self::worst_case_guesses).max().unwrap_or(0),
        }
    }
}

/// Reusable minimax worker pool. A search can occupy the entire pool even when
/// there is only one GameState to evaluate. No nested worker pools are created.
pub struct MinimaxSolver {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
    feedback: Option<Arc<FeedbackTable>>,
}

impl MinimaxSolver {
    /// Creates a fixed pool; workers must be greater than zero.
    pub fn new(workers: usize) -> Self {
        assert!(workers > 0, "worker count must be greater than zero");
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..workers)
            .map(|index| {
                let receiver = Arc::clone(&receiver);
                thread::Builder::new()
                    .name(format!("minimax-{index}"))
                    .spawn(move || loop {
                        // Release the receive lock before running the search.
                        let job = receiver.lock().unwrap().recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    })
                    .expect("failed to start minimax worker")
            })
            .collect();
        Self { sender: Some(sender), workers, feedback: None }
    }

    /// Reuse the table when the allowed words are reordered or narrowed.
    /// An expanded vocabulary starts a new table over the union. Existing
    /// searches retain their old table through Arc; subsequent searches share
    /// the expanded table. Normal searches with a fixed opening reuse one table.
    fn feedback_table(&mut self, words: &[Word]) -> Arc<FeedbackTable> {
        if let Some(table) = self.feedback.as_ref() {
            if words.iter().all(|word| table.words.binary_search(word).is_ok()) {
                return Arc::clone(table);
            }
        }
        let mut vocabulary = words.to_vec();
        if let Some(table) = self.feedback.as_ref() {
            vocabulary.extend_from_slice(&table.words);
        }
        vocabulary.sort_unstable();
        vocabulary.dedup();
        let table = Arc::new(FeedbackTable::new(vocabulary));
        self.feedback = Some(Arc::clone(&table));
        table
    }

    /// Returns the actual worst-case length of the first strategy found.
    /// This need not be the minimum achievable length.
    pub fn strategy_guesses(
        &mut self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<usize> {
        self.decision_tree(allowed, candidates, max_depth)
            .map(|tree| tree.worst_case_guesses())
    }

    /// Finds a strategy within max_depth and reconstructs it from saved proofs.
    /// Accepts the first success; worker schedules may select different strategies.
    /// Does not search smaller budgets to minimize the strategy's length.
    pub fn decision_tree(
        &mut self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<DecisionTree> {
        let (search, candidates, depth) = self.prove(allowed, candidates, max_depth)?;
        Some(search.build_tree(&candidates, depth))
    }

    fn prove(
        &mut self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<(Arc<Search>, Arc<Vec<WordIndex>>, usize)> {
        let mut words = allowed.to_vec();
        words.sort_unstable();
        words.dedup();
        let mut candidates: Vec<WordIndex> = candidates.iter()
            .map(|word| words.binary_search(word).ok().map(WordIndex))
            .collect::<Option<_>>()?;
        candidates.sort_unstable();
        candidates.dedup();
        if candidates.is_empty() {
            return None;
        }

        let feedback = self.feedback_table(&words);
        let feedback_indices = words.iter()
            .map(|word| FeedbackWordIndex(feedback.words.binary_search(word).unwrap())).collect();
        let search = Arc::new(Search {
            words, feedback, feedback_indices, memo: Mutex::new(HashMap::new()),
        });
        let candidates = Arc::new(candidates);
        // Try candidate guesses first, but retain every allowed probe word.
        let guesses: Vec<_> = candidates.iter().copied().chain(
            (0..search.words.len()).map(WordIndex).filter(|guess| candidates.binary_search(guess).is_err())
        ).collect();

        let depth = max_depth;
        if candidates.len() <= depth {
            return Some((search, candidates, depth));
        }
        if depth <= 1 || candidates.len() > capacity(depth) {
            return None;
        }
        let found = Arc::new(AtomicBool::new(false));
        let (completed, results) = mpsc::channel();
        let mut submitted = 0;
        for batch in guesses.chunks(GUESSES_PER_JOB) {
            let batch = batch.to_vec();
            let search = Arc::clone(&search);
            let candidates = Arc::clone(&candidates);
            let found = Arc::clone(&found);
            let completed = completed.clone();
            self.sender.as_ref().unwrap().send(Box::new(move || {
                // Report panics to the caller without silently losing a worker
                // or leaving the caller waiting forever for job completion.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for guess in batch {
                        if found.load(Ordering::Relaxed) {
                            break;
                        }
                        if search.guess_works(&candidates, depth, guess, &found) == Some(true) {
                            search.memo.lock().unwrap().insert((candidates.to_vec(), depth), Some(guess));
                            found.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }));
                if outcome.is_err() {
                    found.store(true, Ordering::Relaxed);
                }
                let _ = completed.send(outcome);
            })).expect("minimax worker queue disconnected");
            submitted += 1;
        }
        drop(completed);
        let mut panic = None;
        // Drain every job before returning. In-flight
        // branches check found cooperatively; queued batches skip their work.
        for _ in 0..submitted {
            if let Err(payload) = results.recv().expect("minimax job failed to report") {
                panic = Some(payload);
            }
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        if found.load(Ordering::Relaxed) {
            return Some((search, candidates, depth));
        }
        None
    }
}

impl Drop for MinimaxSolver {
    fn drop(&mut self) {
        // Closing the queue wakes idle workers. All searches drain their jobs.
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
pub(crate) fn strategy_guesses(allowed: &[Word], candidates: &[Word], depth: usize) -> Option<usize> {
    MinimaxSolver::new(1).strategy_guesses(allowed, candidates, depth)
}

struct Search {
    words: Vec<Word>,
    feedback: Arc<FeedbackTable>,
    // Indexed by WordIndex; values refer to the shared feedback vocabulary.
    feedback_indices: Vec<FeedbackWordIndex>,
    // Locks cover lookups and inserts only, never scoring or recursive search.
    // Key: the game state as the list of remaining candidate words not ruled out,
    //      and the remaining allowed decision tree depth.
    // Value: Some(guess) is a complete winning proof; None is a proven failure.
    memo: Mutex<HashMap<(Vec<WordIndex>, usize), Option<WordIndex>>>,
}

impl Search {
    /// Replays recorded guesses without running minimax again. The sequential
    /// candidate shortcut also has a constructive proof: guess a candidate and
    /// continue on its smaller feedback groups until the target is hit.
    fn build_tree(&self, candidates: &[WordIndex], depth: usize) -> DecisionTree {
        assert!(depth > 0 && !candidates.is_empty());
        let guess = if candidates.len() <= depth {
            candidates[0]
        } else {
            self.memo.lock().unwrap().get(&(candidates.to_vec(), depth))
                .copied().flatten().expect("successful search must retain a winning guess")
        };
        let word = self.words[guess.0];
        let mut groups: HashMap<Result, Vec<WordIndex>> = HashMap::new();
        for &target in candidates {
            groups.entry(self.pattern(guess, target))
                .or_default().push(target);
        }
        let branches = groups.into_iter().map(|(result, group)| {
            let next = if result == Result::ALL_GREEN {
                DecisionTree::Solved
            } else {
                self.build_tree(&group, depth - 1)
            };
            (result, next)
        }).collect();
        DecisionTree::Guess { word, branches }
    }

    fn pattern(&self, guess: WordIndex, target: WordIndex) -> Result {
        self.feedback.pattern(self.feedback_indices[guess.0], self.feedback_indices[target.0])
    }

    // A guess_works if we can_solve for all of the possible Result patterns we might get back if we guess this word.
    // Mutually recursive with can_solve.
    fn guess_works(&self, candidates: &[WordIndex], depth: usize, guess: WordIndex, stop: &AtomicBool) -> Option<bool> {
        if stop.load(Ordering::Relaxed) {
            return None;
        }

        // Add the word index for each possible target word into `groups` at the index that represents the Result
        // pattern we get for guessing the guess word.
        let mut groups: Vec<Vec<WordIndex>> = vec![Vec::new(); Result::COUNT];
        let limit = capacity(depth - 1);
        for &target in candidates {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let pattern = self.pattern(guess, target);
            if pattern != Result::ALL_GREEN {
                groups[pattern.index()].push(target);
                if groups[pattern.index()].len() > limit || groups[pattern.index()].len() == candidates.len() {
                    return Some(false);
                }
            }
        }
        groups.retain(|group| !group.is_empty());
        groups.sort_unstable_by_key(|group| std::cmp::Reverse(group.len()));
        for group in groups {
            if !self.can_solve(&group, depth - 1, stop)? {
                return Some(false);
            }
        }
        Some(true)
    }

    // Can we solve all possible target words with some (any) choice of the candidate words as our next guess?
    // Mutually recursive with guess_works.
    // None means cancelled, never a proof of failure and never memoized.
    fn can_solve(&self, candidates: &[WordIndex], depth: usize, stop: &AtomicBool) -> Option<bool> {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        if candidates.len() <= depth {
            return Some(true);
        }
        if depth <= 1 || candidates.len() > capacity(depth) {
            return Some(false);
        }
        let key = (candidates.to_vec(), depth);
        if let Some(&answer) = self.memo.lock().unwrap().get(&key) {
            return Some(answer.is_some());
        }

        let child_capacity = capacity(depth - 1);
        let mut ranked = Vec::new();
        for guess in (0..self.words.len()).map(WordIndex) {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let mut counts = [0usize; Result::COUNT];
            let mut largest = 0;
            for &target in candidates {
                let pattern = self.pattern(guess, target);
                if pattern != Result::ALL_GREEN {
                    counts[pattern.index()] += 1;
                    largest = largest.max(counts[pattern.index()]);
                    if largest > child_capacity {
                        break;
                    }
                }
            }
            if largest > child_capacity || largest == candidates.len() {
                continue;
            }
            if largest < depth {
                self.memo.lock().unwrap().insert(key, Some(guess));
                return Some(true);
            }
            let squares: usize = counts.iter().map(|&count| count * count).sum();
            ranked.push((largest, squares, guess));
        }
        ranked.sort_unstable();
        for (_, _, guess) in ranked {
            if self.guess_works(candidates, depth, guess, stop)? {
                self.memo.lock().unwrap().insert(key, Some(guess));
                return Some(true);
            }
        }
        self.memo.lock().unwrap().insert(key, None);
        Some(false)
    }
}

// One winning target plus at most 242 non-winning feedback groups per guess.
fn capacity(depth: usize) -> usize {
    (0..depth).fold(0usize, |n, _| n.saturating_mul(Result::COUNT - 1).saturating_add(1))
}

#[cfg(test)]
#[path = "solver_tests.rs"]
mod tests;
