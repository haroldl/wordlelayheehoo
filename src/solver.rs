//! Bounded parallel strategy search over alternating guess and feedback nodes.
//! A feedback node needs one winning next guess; a guess node needs a solution
//! for every reachable feedback Result. The coordinator owns that graph and
//! cancels losing subtrees. Workers never wait for child jobs in their pool.
//! A bounded frontier exposes deeper work; small subproblems use local DFS.
//! Successful searches retain their decision trees instead of searching again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use crate::words::{LetterResult, Result, Word};

#[path = "search_graph.rs"]
mod search_graph;

/// Cancellation follows the search ancestry, without affecting other guesses
/// or required feedback branches outside the cancelled subtree.
#[derive(Default)]
struct Cancellation {
    stopped: AtomicBool,
    parent: Option<Arc<Self>>,
}

impl Cancellation {
    fn child(parent: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self { stopped: AtomicBool::new(false), parent: Some(Arc::clone(parent)) })
    }

    fn cancel(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        let mut scope = Some(self);
        while let Some(current) = scope {
            if current.stopped.load(Ordering::Relaxed) {
                return true;
            }
            scope = current.parent.as_deref();
        }
        false
    }
}

/// Cancellation is deliberately distinct from a proof of impossibility.
#[derive(Debug, PartialEq, Eq)]
enum SearchOutcome<T> {
    Solved(T),
    Impossible,
    Cancelled,
}

type FeedbackGroups = Vec<(Result, Vec<WordIndex>)>;

/// Position in one Search's sorted, deduplicated list of allowed words.
/// This is not an index into the global WORDS dictionary, nor is it portable
/// between searches with different allowed words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
struct WordIndex(usize);

/// Only positive feedback constrains hard-mode guesses. Gold letters may stay
/// in the same position; grey letters impose no exclusions or upper counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
struct HardModeHints {
    greens: [u8; 5],
    minimum_counts: [u8; 26],
}

impl HardModeHints {
    fn after_feedback(mut self, guess: Word, result: Result) -> Self {
        let mut counts = [0u8; 26];
        for (position, feedback) in result.to_letters().into_iter().enumerate() {
            let letter = guess.as_bytes()[position];
            if feedback == LetterResult::Green {
                self.greens[position] = letter;
            }
            if feedback != LetterResult::Grey {
                counts[usize::from(letter - b'a')] += 1;
            }
        }
        for (minimum, count) in self.minimum_counts.iter_mut().zip(counts) {
            *minimum = (*minimum).max(count);
        }
        self
    }

    fn allows(self, guess: Word) -> bool {
        let mut counts = [0u8; 26];
        for (&letter, green) in guess.as_bytes().iter().zip(self.greens) {
            if green != 0 && letter != green {
                return false;
            }
            counts[usize::from(letter - b'a')] += 1;
        }
        counts.iter().zip(self.minimum_counts).all(|(&count, minimum)| count >= minimum)
    }
}

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

/// Reusable worker pool. A per-search coordinator schedules both opening and
/// deeper subproblems, keeping workers free to execute rather than await jobs.
pub struct MinimaxSolver {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
    feedback: Option<Arc<FeedbackTable>>,
    hard_mode: bool,
}

impl MinimaxSolver {
    /// Creates a fixed pool; workers must be greater than zero.
    /// Hard mode restricts guesses using positive feedback along each branch.
    pub fn new(workers: usize, hard_mode: bool) -> Self {
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
        Self { sender: Some(sender), workers, feedback: None, hard_mode }
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
    #[cfg(test)]
    pub fn strategy_guesses(
        &mut self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<usize> {
        self.decision_tree(allowed, candidates, max_depth)
            .map(|tree| tree.worst_case_guesses())
    }

    /// Accepts the first proven strategy within max_depth, retaining its tree.
    /// Worker schedules may select different strategies; length is not minimized.
    pub fn decision_tree(
        &mut self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<DecisionTree> {
        let (search, candidates) = self.prepare(allowed, candidates)?;
        if max_depth == 0 || candidates.len() > capacity(max_depth) {
            return None;
        }
        search_graph::Scheduler::new(
            search, candidates, max_depth, self.hard_mode.then(HardModeHints::default),
            self.sender.as_ref().unwrap(), self.workers.len(),
        ).run()
    }

    fn prepare(
        &mut self,
        allowed: &[Word],
        candidates: &[Word],
    ) -> Option<(Arc<Search>, Arc<Vec<WordIndex>>)> {
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
        let search = Arc::new(Search { words, feedback, feedback_indices });
        Some((search, Arc::new(candidates)))
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
    MinimaxSolver::new(1, false).strategy_guesses(allowed, candidates, depth)
}

struct Search {
    words: Vec<Word>,
    feedback: Arc<FeedbackTable>,
    // Indexed by WordIndex; values refer to the shared feedback vocabulary.
    feedback_indices: Vec<FeedbackWordIndex>,
}

impl Search {
    fn pattern(&self, guess: WordIndex, target: WordIndex) -> Result {
        self.feedback.pattern(self.feedback_indices[guess.0], self.feedback_indices[target.0])
    }

    /// Partition a guess into required feedback branches, checking cheap bounds
    /// before any descendants are scheduled. Largest groups are tried first.
    fn partition(&self, candidates: &[WordIndex], depth: usize, guess: WordIndex, stop: &Cancellation) -> SearchOutcome<FeedbackGroups> {
        if stop.is_cancelled() {
            return SearchOutcome::Cancelled;
        }
        if depth == 0 {
            return SearchOutcome::Impossible;
        }
        let mut groups = vec![Vec::new(); Result::COUNT];
        let limit = capacity(depth - 1);
        for &target in candidates {
            if stop.is_cancelled() {
                return SearchOutcome::Cancelled;
            }
            let pattern = self.pattern(guess, target);
            let group = &mut groups[pattern.index()];
            group.push(target);
            if pattern != Result::ALL_GREEN
                && (group.len() > limit || group.len() == candidates.len())
            {
                return SearchOutcome::Impossible;
            }
        }
        let mut groups: FeedbackGroups = groups.into_iter().filter(|group| !group.is_empty())
            .map(|group| (self.pattern(guess, group[0]), group)).collect();
        groups.sort_unstable_by_key(|(_, group)| std::cmp::Reverse(group.len()));
        SearchOutcome::Solved(groups)
    }

    /// Local DFS retains only successful choices, including their witnesses.
    fn solve(&self, candidates: &[WordIndex], depth: usize, hints: Option<HardModeHints>, stop: &Cancellation) -> SearchOutcome<DecisionTree> {
        let guesses = match self.rank_guesses(candidates, depth, hints, stop) {
            SearchOutcome::Solved(guesses) => guesses,
            SearchOutcome::Impossible => return SearchOutcome::Impossible,
            SearchOutcome::Cancelled => return SearchOutcome::Cancelled,
        };
        for guess in guesses {
            match self.solve_guess(candidates, depth, hints, guess, stop) {
                SearchOutcome::Impossible => continue,
                outcome => return outcome,
            }
        }
        SearchOutcome::Impossible
    }

    fn solve_guess(&self, candidates: &[WordIndex], depth: usize, hints: Option<HardModeHints>, guess: WordIndex, stop: &Cancellation) -> SearchOutcome<DecisionTree> {
        let groups = match self.partition(candidates, depth, guess, stop) {
            SearchOutcome::Solved(groups) => groups,
            SearchOutcome::Impossible => return SearchOutcome::Impossible,
            SearchOutcome::Cancelled => return SearchOutcome::Cancelled,
        };
        let word = self.words[guess.0];
        let mut branches = HashMap::new();
        for (result, group) in groups {
            if stop.is_cancelled() {
                return SearchOutcome::Cancelled;
            }
            let tree = if result == Result::ALL_GREEN {
                DecisionTree::Solved
            } else {
                let hints = hints.map(|hints| hints.after_feedback(word, result));
                match self.solve(&group, depth - 1, hints, stop) {
                    SearchOutcome::Solved(tree) => tree,
                    SearchOutcome::Impossible => return SearchOutcome::Impossible,
                    SearchOutcome::Cancelled => return SearchOutcome::Cancelled,
                }
            };
            branches.insert(result, tree);
        }
        SearchOutcome::Solved(DecisionTree::Guess { word, branches })
    }

    /// Candidate shortcuts remain legal in hard mode because every remaining
    /// target satisfies the accumulated hints. Other guesses use the existing
    /// largest-group / sum-of-squares heuristic.
    fn rank_guesses(&self, candidates: &[WordIndex], depth: usize, hints: Option<HardModeHints>, stop: &Cancellation) -> SearchOutcome<Vec<WordIndex>> {
        if stop.is_cancelled() {
            return SearchOutcome::Cancelled;
        }
        if candidates.is_empty() || depth == 0 {
            return SearchOutcome::Impossible;
        }
        if candidates.len() <= depth {
            return SearchOutcome::Solved(vec![candidates[0]]);
        }
        if depth <= 1 || candidates.len() > capacity(depth) {
            return SearchOutcome::Impossible;
        }
        let child_capacity = capacity(depth - 1);
        let mut ranked = Vec::new();
        for guess in (0..self.words.len()).map(WordIndex) {
            if stop.is_cancelled() {
                return SearchOutcome::Cancelled;
            }
            if hints.is_some_and(|hints| !hints.allows(self.words[guess.0])) {
                continue;
            }
            let mut counts = [0usize; Result::COUNT];
            let mut largest = 0;
            for &target in candidates {
                if stop.is_cancelled() {
                    return SearchOutcome::Cancelled;
                }
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
                return SearchOutcome::Solved(vec![guess]);
            }
            let squares: usize = counts.iter().map(|&count| count * count).sum();
            ranked.push((largest, squares, guess));
        }
        ranked.sort_unstable();
        if ranked.is_empty() {
            SearchOutcome::Impossible
        } else {
            SearchOutcome::Solved(ranked.into_iter().map(|(_, _, guess)| guess).collect())
        }
    }
}

// One winning target plus at most 242 non-winning feedback groups per guess.
fn capacity(depth: usize) -> usize {
    (0..depth).fold(0usize, |n, _| n.saturating_mul(Result::COUNT - 1).saturating_add(1))
}

#[cfg(test)]
#[path = "solver_tests.rs"]
mod tests;
