use super::*;
use crate::{Word, solver::MinimaxSolver};

const TARGETS: [&str; 8] = [
    "aaaaa", "aaaab", "baaaa", "baaab", "caaaa", "caaab", "daaaa", "daaab",
];

fn fixture(pool: &mut MinimaxSolver, probe: bool) -> Scheduler<'_> {
    let targets: Vec<_> = TARGETS.into_iter().map(|word| Word::new(word).unwrap()).collect();
    let mut allowed = targets.clone();
    if probe {
        allowed.push(Word::new("bcddd").unwrap());
    }
    let (search, candidates) = pool.prepare(&allowed, &targets).unwrap();
    Scheduler::new(search, candidates, 3, None, pool.sender.as_ref().unwrap(), pool.workers.len())
}

fn root_guesses(graph: &Scheduler<'_>) -> Vec<GuessId> {
    let mut guesses: Vec<_> = graph.results[&graph.root].active.iter().copied().collect();
    guesses.sort_by_key(|id| graph.guesses[id].word);
    guesses
}

// Deliver real search results in a deterministic order, independently of OS
// thread scheduling. Remove the corresponding queued task as dispatch would.
fn partition(graph: &mut Scheduler<'_>, id: GuessId) {
    graph.pending.retain(|task| *task != Task::Partition(id));
    let node = &graph.guesses[&id];
    let position = &graph.results[&node.parent].position;
    let outcome = graph.search.partition(&position.candidates, position.depth, node.word, &node.scope);
    graph.complete(Task::Partition(id), Completion::Partitioned(outcome));
}

fn solve(graph: &mut Scheduler<'_>, id: ResultId) {
    graph.pending.retain(|task| *task != Task::Solve(id));
    let node = &graph.results[&id];
    let position = &node.position;
    let outcome = graph.search.solve(&position.candidates, position.depth, position.hints, &node.scope);
    graph.complete(Task::Solve(id), Completion::Solved(outcome));
}

fn verify_targets(tree: &DecisionTree) {
    for target in TARGETS.into_iter().map(|word| Word::new(word).unwrap()) {
        let mut node = tree;
        let mut played = HashSet::new();
        while let Some(guess) = node.guess() {
            assert!(played.insert(guess), "repeated guess");
            assert!(played.len() <= 3, "exceeded guess budget");
            node = node.after_result(Result::from_guess(guess, target)).expect("missing feedback");
        }
        assert_eq!(node, &DecisionTree::Solved);
        assert!(played.contains(&target), "target was never guessed");
    }
}

#[test]
fn winning_guess_cancels_competing_descendants_and_ignores_late_completions() {
    let mut pool = MinimaxSolver::new(4, false);
    let mut graph = fixture(&mut pool, true);
    graph.local_target_limit = 0;
    let roots = root_guesses(&graph);
    let (winner, competitor) = (roots[0], roots[1]);

    // Expand the competitor two levels, with both queued work and a scope a
    // worker could already hold. The eventual root winner must cancel all of it.
    partition(&mut graph, competitor);
    let child = *graph.guesses[&competitor].required.iter()
        .find(|id| graph.results[id].expanded).unwrap();
    graph.pending.retain(|task| *task != Task::Rank(child));
    let node = &graph.results[&child];
    let ranked = graph.search.rank_guesses(
        &node.position.candidates, node.position.depth, node.position.hints, &node.scope,
    );
    graph.complete(Task::Rank(child), Completion::Ranked(ranked));
    let descendant = *graph.results[&child].active.iter().next().unwrap();
    partition(&mut graph, descendant);
    let leaf = *graph.guesses[&descendant].required.iter().next().unwrap();
    let processing_scope = Arc::clone(&graph.results[&leaf].scope);
    let competitor_scope = Arc::clone(&graph.guesses[&competitor].scope);

    graph.local_target_limit = usize::MAX;
    partition(&mut graph, winner);
    let required: Vec<_> = graph.guesses[&winner].required.iter().copied().collect();
    solve(&mut graph, required[0]);
    // Solving one Result does not cancel other required Results or prove the guess.
    assert!(graph.outcome.is_none());
    for id in &required[1..] {
        assert!(!graph.results[id].scope.is_cancelled());
    }
    for id in required.into_iter().skip(1) {
        solve(&mut graph, id);
    }
    assert!(processing_scope.is_cancelled());
    assert!(competitor_scope.is_cancelled());
    assert!(graph.results.is_empty());
    assert!(graph.guesses.is_empty());
    assert!(graph.pending.is_empty());
    assert_eq!(graph.expanded_results, 0);
    verify_targets(graph.outcome.as_ref().unwrap().as_ref().unwrap());

    let before = graph.outcome.clone();
    for outcome in [SearchOutcome::Impossible, SearchOutcome::Cancelled, SearchOutcome::Solved(DecisionTree::Solved)] {
        graph.complete(Task::Solve(leaf), Completion::Solved(outcome));
        assert_eq!(graph.outcome, before);
    }
}

#[test]
fn impossible_result_cancels_its_guess_but_leaves_alternatives_running() {
    let mut pool = MinimaxSolver::new(4, false);
    let mut graph = fixture(&mut pool, false);
    let roots = root_guesses(&graph);
    let (rejected, alternative) = (roots[0], roots[1]);
    let alternative_scope = Arc::clone(&graph.guesses[&alternative].scope);
    partition(&mut graph, rejected);
    let required: Vec<_> = graph.guesses[&rejected].required.iter().copied().collect();
    let scopes: Vec<_> = required.iter().map(|id| Arc::clone(&graph.results[id].scope)).collect();
    // The three targets ending in b cannot fit the two remaining guesses.
    // The other three-target group has useful probes among the allowed words.
    let impossible = *required.iter().find(|id| {
        let candidates = &graph.results[id].position.candidates;
        candidates.len() == 3 && candidates.iter().all(|word| graph.search.words[word.0].as_bytes()[4] == b'b')
    }).unwrap();
    solve(&mut graph, impossible);
    assert!(!graph.guesses.contains_key(&rejected));
    assert!(required.iter().all(|id| !graph.results.contains_key(id)));
    assert!(scopes.iter().all(|scope| scope.is_cancelled()));
    assert!(!alternative_scope.is_cancelled());
    assert!(graph.guesses.contains_key(&alternative));
    assert!(graph.outcome.is_none());
    assert_eq!(graph.results[&graph.root].active.len(), COMPETING_GUESSES);

    // Cancelled queued IDs must be discarded before dispatching any work.
    graph.dispatch();
    assert!(graph.running <= graph.workers);
    assert!(graph.results.values().all(|node| !node.scope.is_cancelled()));
    verify_targets(&graph.run().unwrap());
}

#[test]
fn frontier_budget_falls_back_to_local_dfs_and_pool_handles_deeper_work() {
    for workers in [1, 4] {
        let mut pool = MinimaxSolver::new(workers, false);
        for budget in [1, 2, 8] {
            let mut graph = fixture(&mut pool, true);
            graph.local_target_limit = 0;
            graph.max_expanded_results = budget;
            let opening = root_guesses(&graph)[0];
            partition(&mut graph, opening);
            assert!(graph.expanded_results <= budget);
            assert_eq!(graph.expanded_results, budget.min(3));
            if budget == 1 {
                assert!(graph.pending.iter().all(|task| !matches!(task, Task::Rank(_))));
            } else {
                assert!(graph.pending.iter().any(|task| matches!(task, Task::Rank(_))));
            }
            verify_targets(&graph.run().unwrap());
        }
        // All work must be drained so this search can safely reuse the pool.
        let one = [Word::new("apple").unwrap()];
        assert_eq!(pool.strategy_guesses(&one, &one, 1), Some(1));
    }
}

#[test]
fn expanded_graph_matches_independent_exhaustive_search_in_both_modes() {
    let allowed: Vec<_> = ["aaaaa", "baaaa", "caaaa", "aaaab", "baaab", "caaab", "bcddd"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    for hard_mode in [false, true] {
        for workers in [1, 4] {
            let mut pool = MinimaxSolver::new(workers, hard_mode);
            for mask in 1..(1 << allowed.len()) {
                let targets: Vec<_> = allowed.iter().enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, &word)| word).collect();
                let optimum = if hard_mode {
                    crate::tests::exhaustive_hard_mode(&allowed, &targets)
                } else {
                    crate::tests::exhaustive_minimax(&allowed, &targets)
                };
                for depth in [optimum, optimum - 1] {
                    let (search, candidates) = pool.prepare(&allowed, &targets).unwrap();
                    let mut graph = Scheduler::new(search, candidates, depth,
                        hard_mode.then(HardModeHints::default), pool.sender.as_ref().unwrap(), workers);
                    graph.local_target_limit = 0;
                    let tree = graph.run();
                    if depth < optimum {
                        assert!(tree.is_none(), "subset {mask}, hard mode {hard_mode}");
                    } else {
                        let tree = tree.unwrap();
                        crate::tests::verify_decision_tree(&tree, &allowed, &targets, &HashSet::new(), optimum);
                        assert_eq!(tree.worst_case_guesses(), optimum);
                        if hard_mode {
                            crate::tests::verify_hard_mode_tree(&tree, &mut Vec::new());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn worker_panic_cancels_and_drains_search_without_losing_pool_workers() {
    let mut pool = MinimaxSolver::new(4, false);
    let targets: Vec<_> = TARGETS.into_iter().map(|word| Word::new(word).unwrap()).collect();
    let (search, candidates) = pool.prepare(&targets, &targets).unwrap();
    // Deliberately invalid internal indices force a panic inside worker work.
    let broken = Arc::new(Search {
        words: search.words.clone(), feedback: Arc::clone(&search.feedback), feedback_indices: Vec::new(),
    });
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Scheduler::new(broken, candidates, 3, None, pool.sender.as_ref().unwrap(), 4).run()
    }));
    assert!(outcome.is_err());
    let one = [targets[0]];
    assert_eq!(pool.strategy_guesses(&one, &one, 1), Some(1));
}
