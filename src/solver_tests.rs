use super::{FeedbackWordIndex, MinimaxSolver};
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
    let mut pool = MinimaxSolver::new(1);
    let words = cache_words();
    let table = pool.feedback_table(&words);
    for (guess_id, &guess) in table.words.iter().enumerate() {
        for (target_id, &target) in table.words.iter().enumerate() {
            let expected = crate::Result::from_guess(guess, target);
            assert_eq!(table.pattern(FeedbackWordIndex(guess_id), FeedbackWordIndex(target_id)), expected);
            assert_eq!(table.pattern(FeedbackWordIndex(guess_id), FeedbackWordIndex(target_id)), expected);
            assert_eq!(table.patterns[guess_id * table.words.len() + target_id], expected);
        }
    }
}

#[test]
fn feedback_table_is_reused_across_searches_and_allowed_subsets() {
    let mut pool = MinimaxSolver::new(1);
    let words = cache_words();
    let table = pool.feedback_table(&words);
    let first = table.pattern(FeedbackWordIndex(0), FeedbackWordIndex(1));
    let mut subset = words[..3].to_vec();
    subset.reverse();
    let reused = pool.feedback_table(&subset);
    assert!(Arc::ptr_eq(&table, &reused));
    assert_eq!(reused.pattern(FeedbackWordIndex(0), FeedbackWordIndex(1)), first);
    assert_eq!(pool.strategy_guesses(&subset, &subset[..1], 1), Some(1));
    assert!(Arc::ptr_eq(&table, &pool.feedback_table(&words)));

    // Expansion must not confuse local search indices with table indices.
    let extra = crate::Word::new("zzzzz").unwrap();
    let expanded = pool.feedback_table(&[extra]);
    assert!(!Arc::ptr_eq(&table, &expanded));
    assert_eq!(expanded.words.len(), table.words.len() + 1);
    assert_eq!(pool.strategy_guesses(&[extra], &[extra], 1), Some(1));
    assert_eq!(table.pattern(FeedbackWordIndex(0), FeedbackWordIndex(1)), first);
}

#[test]
fn workers_share_precomputed_feedback_safely() {
    let mut pool = MinimaxSolver::new(10);
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
                    patterns.push(table.pattern(FeedbackWordIndex(guess), FeedbackWordIndex(target)));
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
    let mut pool = MinimaxSolver::new(10);
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

#[test]
fn feedback_table_is_complete_before_any_lookup() {
    let mut pool = MinimaxSolver::new(1);
    let table = pool.feedback_table(&cache_words());
    let expected: Vec<_> = table.words.iter().flat_map(|&guess| {
        table.words.iter().map(move |&target| crate::Result::from_guess(guess, target))
    }).collect();
    assert_eq!(table.patterns, expected);
    assert_eq!(table.patterns.len(), table.words.len().pow(2));
    let empty = super::FeedbackTable::new(Vec::new());
    assert!(empty.patterns.is_empty());
}
