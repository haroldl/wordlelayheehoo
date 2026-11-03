//! Coordinator-owned bipartite search graph. Result nodes choose one guess
//! (OR); guess nodes require every reachable Result (AND). Node identities are
//! local to a search: identical candidate sets can have different hard-mode
//! histories and are deliberately not merged.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::Write;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::{Cancellation, DecisionTree, FeedbackGroups, HardModeHints, Job, Result,
    Search, SearchOutcome, Word, WordIndex};

// Limit speculative guesses per Result and the number of expanded Results.
// Other Results run local DFS. Each expanded guess partitions one candidate
// set, so its required children together hold only that set's candidates.
const COMPETING_GUESSES: usize = 2;
const EXPANDED_RESULTS_PER_WORKER: usize = 2;
const LOCAL_TARGET_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ResultId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GuessId(usize);

#[derive(Clone)]
struct Position {
    candidates: Arc<Vec<WordIndex>>,
    depth: usize,
    hints: Option<HardModeHints>,
}

struct ResultNode {
    // The root represents the initial game state and has no incoming feedback.
    parent: Option<(GuessId, Result)>,
    position: Position,
    scope: Arc<Cancellation>,
    expanded: bool,
    choices: VecDeque<WordIndex>,
    active: HashSet<GuessId>,
}

struct GuessNode {
    parent: ResultId,
    word: WordIndex,
    scope: Arc<Cancellation>,
    partitioned: bool,
    required: HashSet<ResultId>,
    branches: HashMap<Result, DecisionTree>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    Rank(ResultId),
    Solve(ResultId),
    Partition(GuessId),
}

enum Completion {
    Ranked(SearchOutcome<Vec<WordIndex>>),
    Solved(SearchOutcome<DecisionTree>),
    Partitioned(SearchOutcome<FeedbackGroups>),
}

type Event = (Task, std::thread::Result<Completion>);

struct OpeningProgress {
    word: Word,
    solved_results: usize,
    total_results: Option<usize>,
}

struct ProgressSnapshot {
    elapsed: Duration,
    completed_tasks: u64,
    running: usize,
    workers: usize,
    queued: usize,
    rejected_openings: usize,
    untried_openings: usize,
    // None means the root is resolved and only cancelled work remains.
    openings: Option<Vec<OpeningProgress>>,
}

impl fmt::Display for ProgressSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,
            "[{:.1}s] Search: {} tasks completed; {}/{} workers busy; {} queued",
            self.elapsed.as_secs_f64(), self.completed_tasks, self.running, self.workers, self.queued,
        )?;
        if let Some(openings) = &self.openings {
            write!(f, "; openings: {} rejected, {} untried", self.rejected_openings, self.untried_openings)?;
            for opening in openings {
                write!(f, "; {}: ", opening.word.as_str())?;
                if let Some(total) = opening.total_results {
                    write!(f, "{}/{} Results solved", opening.solved_results, total)?;
                } else {
                    write!(f, "partitioning")?;
                }
            }
        } else {
            write!(f, "; draining cancelled tasks")?;
        }
        Ok(())
    }
}

pub(super) struct Scheduler<'a> {
    search: Arc<Search>,
    sender: &'a mpsc::Sender<Job>,
    workers: usize,
    completed: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
    // Pending tasks contain only IDs. Cancelling a subtree immediately frees
    // its graph and inputs; stale IDs are discarded before worker dispatch.
    pending: VecDeque<Task>,
    results: HashMap<ResultId, ResultNode>,
    guesses: HashMap<GuessId, GuessNode>,
    root: ResultId,
    next_result: usize,
    next_guess: usize,
    expanded_results: usize,
    max_expanded_results: usize,
    local_target_limit: usize,
    running: usize,
    outcome: Option<Option<DecisionTree>>,
    progress_interval: Option<Duration>,
    completed_tasks: u64,
    rejected_openings: usize,
}

impl<'a> Scheduler<'a> {
    pub(super) fn new(
        search: Arc<Search>,
        candidates: Arc<Vec<WordIndex>>,
        depth: usize,
        hints: Option<HardModeHints>,
        sender: &'a mpsc::Sender<Job>,
        workers: usize,
    ) -> Self {
        let (completed, events) = mpsc::channel();
        let root = ResultId(0);
        // Preserve candidate-first opening order. When sequential candidate
        // guesses fit the budget, only the first one needs to be considered.
        let choices = if candidates.len() <= depth {
            VecDeque::from([candidates[0]])
        } else {
            candidates.iter().copied().chain(
                (0..search.words.len()).map(WordIndex)
                    .filter(|guess| candidates.binary_search(guess).is_err())
            ).collect()
        };
        let root_node = ResultNode {
            parent: None,
            position: Position { candidates, depth, hints },
            scope: Arc::new(Cancellation::default()),
            expanded: true,
            choices,
            active: HashSet::new(),
        };
        let mut scheduler = Self {
            search, sender, workers, completed, events,
            pending: VecDeque::new(),
            results: HashMap::from([(root, root_node)]),
            guesses: HashMap::new(), root,
            next_result: 1, next_guess: 0,
            expanded_results: 1,
            max_expanded_results: workers.saturating_mul(EXPANDED_RESULTS_PER_WORKER),
            local_target_limit: LOCAL_TARGET_LIMIT,
            running: 0, outcome: None,
            progress_interval: None, completed_tasks: 0, rejected_openings: 0,
        };
        scheduler.fill_guesses(root);
        scheduler
    }

    pub(super) fn with_progress_interval(mut self, interval: Option<Duration>) -> Self {
        self.progress_interval = interval.filter(|interval| !interval.is_zero());
        self
    }

    pub(super) fn run(self) -> Option<DecisionTree> {
        self.run_with_reporter(|progress| {
            // Diagnostics must not fail the search if stderr is closed.
            let _ = writeln!(std::io::stderr().lock(), "{progress}");
        })
    }

    fn run_with_reporter(mut self, mut report: impl FnMut(ProgressSnapshot)) -> Option<DecisionTree> {
        let mut panic = None;
        let started = Instant::now();
        let mut reported_at = started;
        loop {
            self.dispatch();
            if self.running == 0 {
                if let Some(payload) = panic {
                    std::panic::resume_unwind(payload);
                }
                return self.outcome.take().expect("search has unfinished nodes but no work");
            }
            let (task, completion) = if let Some(interval) = self.progress_interval {
                // Check the deadline even with a continuous stream of events.
                // A receive timeout also gives updates while a long DFS job
                // is running and no worker has returned any new information.
                if reported_at.elapsed() >= interval {
                    report(self.progress_snapshot(started.elapsed()));
                    reported_at = Instant::now();
                }
                match self.events.recv_timeout(interval.saturating_sub(reported_at.elapsed())) {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => panic!("worker failed to report completion"),
                }
            } else {
                self.events.recv().expect("worker failed to report completion")
            };
            self.running -= 1;
            self.completed_tasks += 1;
            match completion {
                Ok(completion) if self.outcome.is_none() => self.complete(task, completion),
                Err(payload) => {
                    // Drain already-dispatched work before returning or unwinding,
                    // so a reused pool never overlaps searches or loses workers.
                    if panic.is_none() {
                        panic = Some(payload);
                    }
                    if self.results.contains_key(&self.root) {
                        self.remove_result(self.root);
                    }
                    self.pending.clear();
                    self.outcome = Some(None);
                }
                _ => {}
            }
        }
    }

    fn progress_snapshot(&self, elapsed: Duration) -> ProgressSnapshot {
        let root = self.results.get(&self.root);
        let openings = root.map(|root| {
            let mut openings: Vec<_> = root.active.iter().map(|id| {
                let node = &self.guesses[id];
                OpeningProgress {
                    word: self.search.words[node.word.0],
                    solved_results: node.branches.len(),
                    total_results: node.partitioned.then_some(node.branches.len() + node.required.len()),
                }
            }).collect();
            openings.sort_unstable_by_key(|opening| opening.word);
            openings
        });
        let queued = self.pending.iter().filter(|task| match task {
            Task::Rank(id) | Task::Solve(id) => self.results.contains_key(id),
            Task::Partition(id) => self.guesses.contains_key(id),
        }).count();
        ProgressSnapshot {
            elapsed, completed_tasks: self.completed_tasks, running: self.running, workers: self.workers,
            queued, rejected_openings: self.rejected_openings,
            untried_openings: root.map_or(0, |root| root.choices.len()), openings,
        }
    }

    fn dispatch(&mut self) {
        while self.running < self.workers {
            let Some(task) = self.pending.pop_front() else { break };
            let (position, scope, guess) = match task {
                Task::Rank(id) | Task::Solve(id) => {
                    let Some(node) = self.results.get(&id) else { continue };
                    (node.position.clone(), Arc::clone(&node.scope), None)
                }
                Task::Partition(id) => {
                    let Some(node) = self.guesses.get(&id) else { continue };
                    (self.results[&node.parent].position.clone(), Arc::clone(&node.scope), Some(node.word))
                }
            };
            let search = Arc::clone(&self.search);
            let completed = self.completed.clone();
            self.sender.send(Box::new(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match task {
                        Task::Rank(_) => Completion::Ranked(search.rank_guesses(
                            &position.candidates, position.depth, position.hints, &scope,
                        )),
                        Task::Solve(_) => Completion::Solved(search.solve(
                            &position.candidates, position.depth, position.hints, &scope,
                        )),
                        Task::Partition(_) => Completion::Partitioned(search.partition(
                            &position.candidates, position.depth, guess.unwrap(), &scope,
                        )),
                    }
                }));
                let _ = completed.send((task, outcome));
            })).expect("minimax worker queue disconnected");
            self.running += 1;
        }
    }

    fn complete(&mut self, task: Task, completion: Completion) {
        match (task, completion) {
            (Task::Rank(id), Completion::Ranked(outcome)) if self.results.contains_key(&id) => {
                match outcome {
                    SearchOutcome::Solved(choices) => {
                        self.results.get_mut(&id).unwrap().choices = choices.into();
                        self.fill_guesses(id);
                    }
                    SearchOutcome::Impossible => self.finish_result(id, None),
                    SearchOutcome::Cancelled => panic!("live Result task was cancelled"),
                }
            }
            (Task::Solve(id), Completion::Solved(outcome)) if self.results.contains_key(&id) => {
                match outcome {
                    SearchOutcome::Solved(tree) => self.finish_result(id, Some(tree)),
                    SearchOutcome::Impossible => self.finish_result(id, None),
                    SearchOutcome::Cancelled => panic!("live Result task was cancelled"),
                }
            }
            (Task::Partition(id), Completion::Partitioned(outcome)) if self.guesses.contains_key(&id) => {
                match outcome {
                    SearchOutcome::Solved(groups) => self.add_required_results(id, groups),
                    SearchOutcome::Impossible => self.finish_guess(id, false),
                    SearchOutcome::Cancelled => panic!("live guess task was cancelled"),
                }
            }
            // A cancelled subtree has already been removed. Its late success,
            // failure, or cancellation must never update a surviving ancestor.
            _ => {}
        }
    }

    fn fill_guesses(&mut self, id: ResultId) {
        let node = self.results.get_mut(&id).unwrap();
        while node.active.len() < COMPETING_GUESSES {
            let Some(word) = node.choices.pop_front() else { break };
            let guess = GuessId(self.next_guess);
            self.next_guess += 1;
            self.guesses.insert(guess, GuessNode {
                parent: id, word, scope: Cancellation::child(&node.scope),
                partitioned: false,
                required: HashSet::new(), branches: HashMap::new(),
            });
            node.active.insert(guess);
            self.pending.push_back(Task::Partition(guess));
        }
        if node.active.is_empty() {
            self.finish_result(id, None);
        }
    }

    fn add_required_results(&mut self, guess: GuessId, groups: FeedbackGroups) {
        self.guesses.get_mut(&guess).unwrap().partitioned = true;
        let node = &self.guesses[&guess];
        let position = self.results[&node.parent].position.clone();
        let word = self.search.words[node.word.0];
        let scope = Arc::clone(&node.scope);
        for (feedback, candidates) in groups {
            if feedback == Result::ALL_GREEN {
                self.guesses.get_mut(&guess).unwrap().branches.insert(feedback, DecisionTree::Solved);
                continue;
            }
            let id = ResultId(self.next_result);
            self.next_result += 1;
            let depth = position.depth - 1;
            let expanded = candidates.len() > depth
                && candidates.len() > self.local_target_limit
                && self.expanded_results < self.max_expanded_results;
            self.expanded_results += usize::from(expanded);
            self.results.insert(id, ResultNode {
                parent: Some((guess, feedback)),
                position: Position {
                    candidates: Arc::new(candidates), depth,
                    hints: position.hints.map(|hints| hints.after_feedback(word, feedback)),
                },
                scope: Cancellation::child(&scope), expanded,
                choices: VecDeque::new(), active: HashSet::new(),
            });
            self.guesses.get_mut(&guess).unwrap().required.insert(id);
            self.pending.push_back(if expanded { Task::Rank(id) } else { Task::Solve(id) });
        }
        if self.guesses[&guess].required.is_empty() {
            self.finish_guess(guess, true);
        }
    }

    fn finish_result(&mut self, id: ResultId, tree: Option<DecisionTree>) {
        // One successful guess resolves an OR node. Removing it cancels all
        // competing guesses and descendants before notifying its parent.
        let node = self.remove_result(id);
        let Some((parent, feedback)) = node.parent else {
            self.outcome = Some(tree);
            self.pending.clear();
            return;
        };
        let guess = self.guesses.get_mut(&parent).unwrap();
        guess.required.remove(&id);
        if let Some(tree) = tree {
            guess.branches.insert(feedback, tree);
            if guess.required.is_empty() {
                self.finish_guess(parent, true);
            }
        } else {
            // One impossible Result invalidates an AND node and cancels all
            // other required Results, including any currently processing.
            self.finish_guess(parent, false);
        }
    }

    fn finish_guess(&mut self, id: GuessId, solved: bool) {
        let node = self.remove_guess(id);
        if !solved && node.parent == self.root {
            self.rejected_openings += 1;
        }
        self.results.get_mut(&node.parent).unwrap().active.remove(&id);
        if solved {
            let tree = DecisionTree::Guess {
                word: self.search.words[node.word.0], branches: node.branches,
            };
            self.finish_result(node.parent, Some(tree));
        } else {
            self.fill_guesses(node.parent);
        }
    }

    fn remove_result(&mut self, id: ResultId) -> ResultNode {
        let node = self.results.remove(&id).unwrap();
        node.scope.cancel();
        self.expanded_results -= usize::from(node.expanded);
        for &guess in &node.active {
            self.remove_guess(guess);
        }
        node
    }

    fn remove_guess(&mut self, id: GuessId) -> GuessNode {
        let node = self.guesses.remove(&id).unwrap();
        node.scope.cancel();
        for &result in &node.required {
            self.remove_result(result);
        }
        node
    }
}

#[cfg(test)]
#[path = "search_graph_tests.rs"]
mod tests;
