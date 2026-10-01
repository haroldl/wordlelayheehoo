use super::MinimaxSolver;
use std::collections::HashSet;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

#[test]
fn queue_runs_jobs_on_the_configured_number_of_workers() {
    for count in [1, 10, 20] {
        let pool = MinimaxSolver::new(count);
        let barrier = Arc::new(Barrier::new(count));
        let (sender, receiver) = mpsc::channel();
        for _ in 0..count {
            let barrier = Arc::clone(&barrier);
            let sender = sender.clone();
            pool.sender.as_ref().unwrap().send(Box::new(move || {
                // Each worker must hold a job simultaneously to cross this barrier.
                barrier.wait();
                sender.send(std::thread::current().id()).unwrap();
            })).unwrap();
        }
        let ids: HashSet<_> = (0..count)
            .map(|_| receiver.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(ids.len(), count);
        drop(pool); // Closing an idle queue must wake and join every worker.
    }
}

fn cache_words() -> Vec<crate::Word> {
    // Repeats exercise the green-first and excess-letter rules.
    ["apple", "allee", "eerie", "level", "belle", "aaaaa", "bbbbb"]
        .into_iter().map(|word| crate::Word::new(word).unwrap()).collect()
}

#[test]
fn cached_feedback_matches_scoring_for_every_pair() {
    let pool = MinimaxSolver::new(1);
    let words = cache_words();
    let table = pool.feedback_table(&words);
    for (guess_id, &guess) in table.words.iter().enumerate() {
        for (target_id, &target) in table.words.iter().enumerate() {
            let expected = crate::Result::from_guess(guess, target).as_slice().iter()
                .fold(0usize, |code, &letter| code * 3 + letter as usize);
            assert_eq!(table.pattern(guess_id, target_id), expected);
            assert_eq!(table.pattern(guess_id, target_id), expected);
            assert_eq!(usize::from(table.patterns[guess_id * table.words.len() + target_id]
                .load(super::Ordering::Relaxed)), expected);
        }
    }
}

#[test]
fn feedback_table_is_reused_across_searches_and_allowed_subsets() {
    let pool = MinimaxSolver::new(1);
    let words = cache_words();
    let table = pool.feedback_table(&words);
    let first = table.pattern(0, 1);
    let mut subset = words[..3].to_vec();
    subset.reverse();
    let reused = pool.feedback_table(&subset);
    assert!(Arc::ptr_eq(&table, &reused));
    assert_eq!(reused.pattern(0, 1), first);
    assert_eq!(pool.strategy_guesses(&subset, &subset[..1], 1), Some(1));
    assert!(Arc::ptr_eq(&table, &pool.feedback_table(&words)));

    // Expansion must not confuse local search indices with table indices.
    let extra = crate::Word::new("zzzzz").unwrap();
    let expanded = pool.feedback_table(&[extra]);
    assert!(!Arc::ptr_eq(&table, &expanded));
    assert_eq!(expanded.words.len(), table.words.len() + 1);
    assert_eq!(pool.strategy_guesses(&[extra], &[extra], 1), Some(1));
    assert_eq!(table.pattern(0, 1), first);
}

#[test]
fn workers_share_cold_feedback_cells_safely() {
    let pool = MinimaxSolver::new(10);
    let table = pool.feedback_table(&cache_words());
    let barrier = Arc::new(Barrier::new(10));
    let (sender, receiver) = mpsc::channel();
    for _ in 0..10 {
        let table = Arc::clone(&table);
        let barrier = Arc::clone(&barrier);
        let sender = sender.clone();
        pool.sender.as_ref().unwrap().send(Box::new(move || {
            barrier.wait();
            let mut patterns = Vec::new();
            for guess in 0..table.words.len() {
                for target in 0..table.words.len() {
                    patterns.push(table.pattern(guess, target));
                }
            }
            sender.send(patterns).unwrap();
        })).unwrap();
    }
    let expected = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    for _ in 1..10 {
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), expected);
    }
}

#[test]
fn search_uses_cached_indices_when_allowed_words_are_narrowed() {
    let pool = MinimaxSolver::new(10);
    let vocabulary: Vec<_> = ["aaaaa", "baaaa", "caaaa", "daaaa", "bcddd", "zzzzz"]
        .into_iter().map(|word| crate::Word::new(word).unwrap()).collect();
    let table = pool.feedback_table(&vocabulary);
    let allowed = &vocabulary[1..5];
    let candidates = &vocabulary[1..4];
    let tree = pool.decision_tree(allowed, candidates, 2).unwrap();
    assert_eq!(tree.guess(), Some(vocabulary[4]));
    assert_eq!(tree.worst_case_guesses(), 2);
    assert!(Arc::ptr_eq(&table, &pool.feedback_table(allowed)));
}
