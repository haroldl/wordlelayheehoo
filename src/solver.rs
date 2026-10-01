//! Exact minimax with a reusable, fixed-size worker pool.
//! For each depth bound, queued jobs evaluate batches of proposed first guesses.
//! Each job searches its feedback subtrees recursively; workers share proven
//! subproblem results. Workers never wait for jobs in their own pool, avoiding
//! nested-pool deadlocks and limiting total concurrency to the configured count.
//! Previously guessed words are excluded on entry. Deeper repeated guesses give
//! one non-winning group with every candidate and are rejected as no progress.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use crate::words::{Result, Word};

const PATTERNS: usize = 243;
const ALL_GREEN: usize = PATTERNS - 1;
const GUESSES_PER_JOB: usize = 16;
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
        Self { sender: Some(sender), workers }
    }

    pub fn minimum_guesses(
        &self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<usize> {
        self.prove(allowed, candidates, max_depth).map(|(_, _, depth)| depth)
    }

    /// Finds an optimal worst-case strategy and reconstructs it from saved proofs.
    /// Ties may select different guesses with different worker schedules.
    /// Branches meet the root's optimal budget; they need not each be locally optimal.
    pub fn decision_tree(
        &self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<DecisionTree> {
        let (search, candidates, depth) = self.prove(allowed, candidates, max_depth)?;
        Some(search.build_tree(&candidates, depth))
    }

    fn prove(
        &self,
        allowed: &[Word],
        candidates: &[Word],
        max_depth: usize,
    ) -> Option<(Arc<Search>, Arc<Vec<usize>>, usize)> {
        let mut words = allowed.to_vec();
        words.sort_unstable();
        words.dedup();
        let mut candidates: Vec<usize> = candidates.iter()
            .map(|word| words.binary_search(word).ok())
            .collect::<Option<_>>()?;
        candidates.sort_unstable();
        candidates.dedup();
        if candidates.is_empty() {
            return None;
        }

        let search = Arc::new(Search { words, memo: Mutex::new(HashMap::new()) });
        let candidates = Arc::new(candidates);
        // Try candidate guesses first, but retain every allowed probe word.
        let guesses: Vec<_> = candidates.iter().copied().chain(
            (0..search.words.len()).filter(|guess| candidates.binary_search(guess).is_err())
        ).collect();

        for depth in 1..=max_depth.min(candidates.len()) {
            if candidates.len() <= depth {
                return Some((search, candidates, depth));
            }
            if depth <= 1 || candidates.len() > capacity(depth) {
                continue;
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
            // Drain every job before advancing depth or returning. In-flight
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
pub(crate) fn minimum_guesses(allowed: &[Word], candidates: &[Word], depth: usize) -> Option<usize> {
    MinimaxSolver::new(1).minimum_guesses(allowed, candidates, depth)
}

struct Search {
    words: Vec<Word>,
    // Locks cover lookups and inserts only, never scoring or recursive search.
    // Some(guess) is a complete winning proof; None is a proven failure.
    memo: Mutex<HashMap<(Vec<usize>, usize), Option<usize>>>,
}

impl Search {
    /// Replays recorded guesses without running minimax again. The sequential
    /// candidate shortcut also has a constructive proof: guess a candidate and
    /// continue on its smaller feedback groups until the target is hit.
    fn build_tree(&self, candidates: &[usize], depth: usize) -> DecisionTree {
        assert!(depth > 0 && !candidates.is_empty());
        let guess = if candidates.len() <= depth {
            candidates[0]
        } else {
            self.memo.lock().unwrap().get(&(candidates.to_vec(), depth))
                .copied().flatten().expect("successful search must retain a winning guess")
        };
        let word = self.words[guess];
        let mut groups: HashMap<Result, Vec<usize>> = HashMap::new();
        for &target in candidates {
            groups.entry(Result::from_guess(word, self.words[target]))
                .or_default().push(target);
        }
        let branches = groups.into_iter().map(|(result, group)| {
            let next = if group.len() == 1 && group[0] == guess {
                DecisionTree::Solved
            } else {
                self.build_tree(&group, depth - 1)
            };
            (result, next)
        }).collect();
        DecisionTree::Guess { word, branches }
    }

    fn pattern(&self, guess: usize, target: usize) -> usize {
        Result::from_guess(self.words[guess], self.words[target])
            .as_slice().iter()
            .fold(0, |code, &letter| code * 3 + letter as usize)
    }

    fn guess_works(&self, candidates: &[usize], depth: usize, guess: usize, stop: &AtomicBool) -> Option<bool> {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let mut groups: Vec<Vec<usize>> = vec![Vec::new(); PATTERNS];
        let limit = capacity(depth - 1);
        for &target in candidates {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let pattern = self.pattern(guess, target);
            if pattern != ALL_GREEN {
                groups[pattern].push(target);
                if groups[pattern].len() > limit || groups[pattern].len() == candidates.len() {
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

    // None means cancelled, never a proof of failure and never memoized.
    fn can_solve(&self, candidates: &[usize], depth: usize, stop: &AtomicBool) -> Option<bool> {
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
        for guess in 0..self.words.len() {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let mut counts = [0usize; PATTERNS];
            let mut largest = 0;
            for &target in candidates {
                let pattern = self.pattern(guess, target);
                if pattern != ALL_GREEN {
                    counts[pattern] += 1;
                    largest = largest.max(counts[pattern]);
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
    (0..depth).fold(0usize, |n, _| n.saturating_mul(242).saturating_add(1))
}

#[cfg(test)]
#[path = "solver_tests.rs"]
mod tests;
