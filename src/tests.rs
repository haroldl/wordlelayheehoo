use clap::Parser;
use super::{LetterResult, MAX_GUESSES, Result, Word, WORDS};
use LetterResult::{Gold, Green, Grey};

// Fixed vocabulary for solver tests; independent of the production WORD.LST.
// Includes several words that match apple after the arose/unlit opening.
const TEST_WORDS: [&str; 136] = [
    "apple", "ample", "amble", "addle", "arose", "unlit",
    "about", "above", "abuse", "actor", "acute", "admit",
    "adopt", "adult", "after", "again", "agent", "agree",
    "ahead", "alarm", "album", "alert", "alike", "alive",
    "allow", "alone", "along", "alter", "among", "anger",
    "angle", "angry", "apart", "arena", "argue", "arise",
    "array", "aside", "asset", "audio", "audit", "avoid",
    "awake", "award", "aware", "badly", "baker", "bases",
    "basic", "basis", "beach", "began", "begin", "begun",
    "being", "below", "bench", "billy", "birth", "black",
    "blame", "blind", "block", "blood", "board", "boost",
    "booth", "bound", "brain", "brand", "bread", "break",
    "breed", "brief", "bring", "broad", "broke", "brown",
    "build", "built", "buyer", "cable", "carry", "catch",
    "cause", "chain", "chair", "chart", "chase", "cheap",
    "check", "chest", "chief", "child", "china", "chose",
    "civil", "claim", "class", "clean", "clear", "click",
    "clock", "close", "coach", "coast", "could", "count",
    "court", "cover", "craft", "crash", "cream", "crime",
    "cross", "crowd", "crown", "curve", "cycle", "daily",
    "dance", "dealt", "death", "debut", "delay", "depth",
    "doing", "doubt", "dozen", "draft", "drama", "drawn",
    "dream", "dress", "drill", "drink",
];

#[test]
fn normalizes_case_and_borrows_letters() {
    let word = Word::new("AbCdE").unwrap();
    assert_eq!(word.as_str(), "abcde");
    assert_eq!(word.as_bytes(), b"abcde");
    assert_eq!(Word::new("APPLE"), Word::new("apple"));
    for input in ["APPLE", "aZzZa", "zzzzz"] {
        let word = Word::new(input).unwrap();
        assert_eq!(word.as_str(), input.to_ascii_lowercase());
        assert_eq!(Word::new(word.as_str()), Some(word));
    }
}

#[test]
fn rejects_invalid_strings() {
    for word in ["", "abcd", "abcdef", "abc1e", "ab de", "abcdé", "abcé"] {
        assert_eq!(Word::new(word), None, "accepted {word:?}");
    }
}

#[test]
fn uses_five_bytes_per_word() {
    assert_eq!(std::mem::size_of::<Word>(), 5);
    assert_eq!(std::mem::size_of::<[Word; 10]>(), 50);
}

#[test]
fn generated_words_match_runtime_validation() {
    // The full source text is embedded only in the test executable.
    let expected: Vec<Word> = include_str!("../WORD.LST")
        .lines()
        .filter_map(Word::new)
        .collect();
    assert!(!WORDS.is_empty());
    assert_eq!(WORDS, expected.as_slice());
}

fn assert_feedback(guess: &str, target: &str, expected: [LetterResult; 5]) {
    let result = Result::from_guess(Word::new(guess).unwrap(), Word::new(target).unwrap());
    assert_eq!(result.to_letters(), expected, "guess {guess}, target {target}");
}

#[test]
fn exact_guess_is_all_green() {
    assert_feedback("apple", "apple", [Green; 5]);
}

#[test]
fn absent_letters_are_all_grey() {
    assert_feedback("xxxxx", "apple", [Grey; 5]);
}

#[test]
fn misplaced_letters_are_all_gold() {
    assert_feedback("eabcd", "abcde", [Gold; 5]);
}

#[test]
fn scores_mixed_feedback() {
    assert_feedback("arose", "apple", [Green, Grey, Grey, Grey, Green]);
    assert_feedback("crane", "react", [Gold, Gold, Green, Grey, Gold]);
}

#[test]
fn green_matches_take_priority_over_earlier_duplicate_guesses() {
    assert_feedback("eerie", "apple", [Grey, Grey, Grey, Grey, Green]);
    assert_feedback("allee", "apple", [Green, Gold, Grey, Grey, Green]);
}

#[test]
fn gold_matches_consume_only_available_occurrences_from_left_to_right() {
    assert_feedback("llama", "apple", [Gold, Grey, Gold, Grey, Grey]);
    assert_feedback("ppaaa", "apple", [Gold, Green, Gold, Grey, Grey]);
    assert_feedback("aabbb", "ccaaa", [Gold, Gold, Grey, Grey, Grey]);
}

#[test]
fn scoring_uses_normalized_words() {
    assert_feedback("CrAnE", "REACT", [Gold, Gold, Green, Grey, Gold]);
}

#[test]
fn minimax_counts_the_final_guess_and_respects_budget() {
    let words: Vec<_> = ["aaaaa", "baaaa", "caaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    assert_eq!(super::solver::strategy_guesses(&words, &words[..1], 1), Some(1));
    assert_eq!(super::solver::strategy_guesses(&words, &words[..1], 0), None);
    assert_eq!(super::solver::strategy_guesses(&words, &words, 2), None);
    assert_eq!(super::solver::strategy_guesses(&words, &words, 3), Some(3));
    assert_eq!(super::solver::strategy_guesses(&words, &[], 3), None);
}

#[test]
fn minimax_can_use_a_non_candidate_probe() {
    let candidates: Vec<_> = ["aaaaa", "baaaa", "caaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    let mut allowed = candidates.clone();
    allowed.push(Word::new("bcddd").unwrap());
    // bcddd produces three distinct non-winning results, then the answer
    // must be guessed. Guessing only candidates takes three in the worst case.
    assert_eq!(super::solver::strategy_guesses(&allowed, &candidates, 2), Some(2));
    allowed.push(allowed[0]);
    let mut repeated = candidates.clone();
    repeated.push(candidates[0]);
    assert_eq!(super::solver::strategy_guesses(&allowed, &repeated, 2), Some(2));
}

// Independent exhaustive recurrence for tiny dictionaries, with no memoization,
// depth feasibility search, ranking, or capacity pruning.
fn exhaustive_minimax(allowed: &[Word], candidates: &[Word]) -> usize {
    let mut best = candidates.len();
    for &guess in allowed {
        let mut groups: std::collections::HashMap<Result, Vec<Word>> =
            std::collections::HashMap::new();
        for &target in candidates {
            if guess != target {
                groups.entry(Result::from_guess(guess, target)).or_default().push(target);
            }
        }
        if groups.values().any(|group| group.len() == candidates.len()) {
            continue;
        }
        let worst = groups.values().map(|group| exhaustive_minimax(allowed, group))
            .max().unwrap_or(0);
        best = best.min(1 + worst);
    }
    best
}

#[test]
fn minimax_matches_exhaustive_search_on_every_small_candidate_subset() {
    let allowed: Vec<_> = ["aaaaa", "baaaa", "caaaa", "daaaa", "bcddd"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    for mask in 1..(1 << allowed.len()) {
        let candidates: Vec<_> = allowed.iter().enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, &word)| word).collect();
        let expected = exhaustive_minimax(&allowed, &candidates);
        assert_eq!(
            super::solver::strategy_guesses(&allowed, &candidates, expected),
            Some(expected), "subset {mask}"
        );
        assert_eq!(
            super::solver::strategy_guesses(&allowed, &candidates, expected - 1),
            None, "subset {mask} fits below its optimum"
        );
    }
}

#[test]
fn minimax_searches_multiple_levels_when_guesses_only_eliminate_one_target() {
    let words: Vec<_> = ["aaaaa", "baaaa", "caaaa", "daaaa", "eaaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    // Every miss leaves all other targets indistinguishable, so all five
    // guesses can be necessary. Depth four exercises recursive failed branches.
    assert_eq!(super::solver::strategy_guesses(&words, &words, 4), None);
    assert_eq!(super::solver::strategy_guesses(&words, &words, 5), Some(5));
}

#[test]
fn worker_count_arguments_are_validated() {
    let defaults = super::Args::try_parse_from(["wordlelayheehoo"]).unwrap();
    assert_eq!(defaults.workers.get(), super::DEFAULT_SOLVER_WORKERS);

    for count in [1, 10, 20] {
        let value = count.to_string();
        for flag in ["--workers", "-w"] {
            let args = super::Args::try_parse_from(["wordlelayheehoo", flag, &value]).unwrap();
            assert_eq!(args.workers.get(), count);
        }
    }
    for args in [
        vec!["--workers", "0"],
        vec!["--workers", "-1"],
        vec!["--workers", "abc"],
        vec!["--workers"],
        vec!["--other", "20"],
    ] {
        let command = std::iter::once("wordlelayheehoo").chain(args);
        assert!(super::Args::try_parse_from(command).is_err());
    }
}

#[test]
fn cli_supports_help_and_version() {
    for (flag, kind) in [
        ("--help", clap::error::ErrorKind::DisplayHelp),
        ("--version", clap::error::ErrorKind::DisplayVersion),
    ] {
        let result = super::Args::try_parse_from(["wordlelayheehoo", flag]);
        assert!(matches!(result, Err(error) if error.kind() == kind && error.exit_code() == 0));
    }
}

#[test]
fn queued_minimax_matches_exhaustive_search_with_one_ten_and_twenty_workers() {
    let allowed: Vec<_> = ["aaaaa", "baaaa", "caaaa", "daaaa", "bcddd"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    for workers in [1, 10, 20] {
        let mut solver = super::solver::MinimaxSolver::new(workers);
        for mask in 1..(1 << allowed.len()) {
            let candidates: Vec<_> = allowed.iter().enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, &word)| word).collect();
            let expected = exhaustive_minimax(&allowed, &candidates);
            // Reuse a pool after both success (with cancellation) and failure.
            assert_eq!(solver.strategy_guesses(&allowed, &candidates, expected), Some(expected));
            assert_eq!(solver.strategy_guesses(&allowed, &candidates, expected - 1), None);
        }
    }
}

#[test]
fn queued_minimax_exhausts_multiple_batches_without_a_winning_strategy() {
    // A shared suffix makes each missed candidate eliminate only itself.
    // More than 16 words forces work into multiple queue jobs.
    let words: Vec<_> = (b'a'..=b't').map(|letter| Word::new(&format!("{}zzzz", char::from(letter))).unwrap()).collect();
    for workers in [1, 10, 20] {
        let mut solver = super::solver::MinimaxSolver::new(workers);
        assert_eq!(solver.strategy_guesses(&words, &words, 3), None);
        assert_eq!(solver.strategy_guesses(&words, &words[..1], 1), Some(1));
    }
}

fn verify_decision_tree(
    tree: &super::DecisionTree,
    allowed: &[Word],
    candidates: &[Word],
    played: &std::collections::HashSet<Word>,
    budget: usize,
) {
    let super::DecisionTree::Guess { word, branches } = tree else {
        panic!("an unguessed target needs a guess node");
    };
    assert!(budget > 0);
    assert!(allowed.contains(word));
    let mut played = played.clone();
    assert!(played.insert(*word), "strategy repeated a guess");
    let mut groups: std::collections::HashMap<Result, Vec<Word>> = std::collections::HashMap::new();
    for &target in candidates {
        groups.entry(Result::from_guess(*word, target)).or_default().push(target);
    }
    assert_eq!(branches.len(), groups.len(), "unreachable feedback branch");
    assert_eq!(tree.guess(), Some(*word));
    for (result, targets) in groups {
        let next = tree.after_result(result).expect("missing reachable feedback branch");
        if result == Result::ALL_GREEN {
            assert_eq!(targets, vec![*word]);
            assert_eq!(next, &super::DecisionTree::Solved);
        } else {
            verify_decision_tree(next, allowed, &targets, &played, budget - 1);
        }
    }
}

#[test]
fn decision_trees_cover_all_targets_at_the_exhaustive_optimum() {
    let allowed: Vec<_> = ["aaaaa", "baaaa", "caaaa", "daaaa", "bcddd"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    for workers in [1, 10, 20] {
        let mut solver = super::solver::MinimaxSolver::new(workers);
        for mask in 1..(1 << allowed.len()) {
            let candidates: Vec<_> = allowed.iter().enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, &word)| word).collect();
            let optimum = exhaustive_minimax(&allowed, &candidates);
            let tree = solver.decision_tree(&allowed, &candidates, optimum).unwrap();
            assert_eq!(tree.worst_case_guesses(), optimum);
            verify_decision_tree(&tree, &allowed, &candidates, &std::collections::HashSet::new(), optimum);
            assert_eq!(solver.decision_tree(&allowed, &candidates, optimum - 1), None);
        }
    }
}

#[test]
fn decision_tree_includes_non_candidate_probe_and_final_correct_guess() {
    let candidates: Vec<_> = ["aaaaa", "baaaa", "caaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    let probe = Word::new("bcddd").unwrap();
    let mut allowed = candidates.clone();
    allowed.push(probe);
    let mut solver = super::solver::MinimaxSolver::new(10);
    let tree = solver.decision_tree(&allowed, &candidates, 2).unwrap();
    assert_eq!(tree.guess(), Some(probe));
    assert_eq!(tree.after_result(Result::ALL_GREEN), None);
    verify_decision_tree(&tree, &allowed, &candidates, &std::collections::HashSet::new(), 2);
}

#[test]
fn solver_accepts_a_longer_strategy_when_it_fits_the_budget() {
    let candidates: Vec<_> = ["aaaaa", "baaaa", "caaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    let mut allowed = candidates.clone();
    allowed.push(Word::new("bcddd").unwrap());
    for workers in [1, 10, 20] {
        let mut solver = super::solver::MinimaxSolver::new(workers);
        // A probe can solve in two, but sequential candidate guesses fit six.
        assert_eq!(solver.strategy_guesses(&allowed, &candidates, 2), Some(2));
        let tree = solver.decision_tree(&allowed, &candidates, MAX_GUESSES).unwrap();
        assert_eq!(tree.guess(), Some(candidates[0]));
        assert_eq!(tree.worst_case_guesses(), 3);
        assert_eq!(solver.strategy_guesses(&allowed, &candidates, MAX_GUESSES), Some(3));
        verify_decision_tree(&tree, &allowed, &candidates,
            &std::collections::HashSet::new(), MAX_GUESSES);
    }
}

#[test]
fn output_file_argument_accepts_long_and_short_options() {
    assert!(super::Args::try_parse_from(["wordlelayheehoo"]).unwrap().output.is_none());
    for flag in ["--output", "-o"] {
        let args = super::Args::try_parse_from(["wordlelayheehoo", flag, "strategy.json"]).unwrap();
        assert_eq!(args.output.unwrap(), std::path::PathBuf::from("strategy.json"));
        assert!(super::Args::try_parse_from(["wordlelayheehoo", flag]).is_err());
    }
}

#[test]
fn decision_tree_json_preserves_guesses_feedback_and_solved_leaves() {
    use super::DecisionTree;
    let tree = DecisionTree::Guess {
        word: Word::new("arose").unwrap(),
        branches: std::collections::HashMap::from([
            (Result::new([Grey, Grey, Gold, Green, Grey]), DecisionTree::Guess {
                word: Word::new("boost").unwrap(),
                branches: std::collections::HashMap::from([
                    (Result::ALL_GREEN, DecisionTree::Solved),
                ]),
            }),
            (Result::ALL_GREEN, DecisionTree::Solved),
        ]),
    };
    let mut json = Vec::new();
    tree.write_json(&mut json).unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&json).unwrap(), serde_json::from_str::<serde_json::Value>(r#"{
  "guess": "arose",
  "branches": {
    "*****": {"solved": true},
    "__+*_": {
      "guess": "boost",
      "solved": true
    }
  }
}
"#).unwrap());
    let mut json = Vec::new();
    DecisionTree::Solved.write_json(&mut json).unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&json).unwrap(), serde_json::json!({"solved": true}));

    let mut full: &mut [u8] = &mut [];
    assert_eq!(tree.write_json(&mut full).unwrap_err().kind(), std::io::ErrorKind::WriteZero);
}

#[test]
fn sample_percentage_arguments_are_validated() {
    assert_eq!(super::Args::try_parse_from(["wordlelayheehoo"]).unwrap().sample_percent, 100.0);
    for value in ["0.01", "10", "99.5", "100"] {
        let args = super::Args::try_parse_from(["wordlelayheehoo", "--sample-percent", value]).unwrap();
        assert_eq!(args.sample_percent, value.parse::<f64>().unwrap());
    }
    for value in ["0", "-1", "100.1", "NaN", "inf", "abc"] {
        assert!(super::Args::try_parse_from(["wordlelayheehoo", "--sample-percent", value]).is_err());
    }
    assert!(super::Args::try_parse_from(["wordlelayheehoo", "--sample-percent"]).is_err());
}

#[test]
fn sampling_preserves_full_dictionary_and_selects_without_replacement() {
    use rand::SeedableRng;
    let words: Vec<_> = ["apple", "arose", "unlit", "belle", "boost", "crane", "slate"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
    assert_eq!(super::sampling::sample_words(&words, 100.0, &mut rng), words);
    assert!(super::sampling::sample_words(&[], 10.0, &mut rng).is_empty());
    assert_eq!(super::sampling::sample_words(&words, 0.001, &mut rng).len(), 1);
    let sample = super::sampling::sample_words(&words, 30.0, &mut rng);
    assert_eq!(sample.len(), 3); // ceil(7 * .3)
    assert!(sample.iter().all(|word| words.contains(word)));
    assert_eq!(sample.iter().collect::<std::collections::HashSet<_>>().len(), sample.len());
    let mut repeated_rng = rand::rngs::SmallRng::seed_from_u64(42);
    let mut original_rng = rand::rngs::SmallRng::seed_from_u64(42);
    assert_eq!(
        super::sampling::sample_words(&words, 50.0, &mut repeated_rng),
        super::sampling::sample_words(&words, 50.0, &mut original_rng),
    );
}

#[test]
fn saved_trees_round_trip_through_json() {
    let words: Vec<_> = ["apple", "ample", "arose"].into_iter()
        .map(|text| Word::new(text).unwrap()).collect();
    let tree = super::solver::MinimaxSolver::new(1).decision_tree(&words, &words, 3).unwrap();
    let mut bytes = Vec::new();
    tree.write_json(&mut bytes).unwrap();
    assert_eq!(super::DecisionTree::read_json(bytes.as_slice()).unwrap(), tree);
    let mut again = Vec::new();
    super::DecisionTree::read_json(bytes.as_slice()).unwrap().write_json(&mut again).unwrap();
    assert_eq!(bytes, again);
}

#[test]
fn loads_compact_legacy_and_solved_tree_nodes() {
    let compact = br#"{"guess":"apple","solved":true}"#;
    let legacy = br#"{"guess":"apple","branches":{"*****":{"solved":true}}}"#;
    let tree = super::DecisionTree::read_json(&compact[..]).unwrap();
    assert_eq!(tree.worst_case_guesses(), 1);
    assert_eq!(tree, super::DecisionTree::read_json(&legacy[..]).unwrap());
    assert_eq!(super::DecisionTree::read_json(&b"{\"solved\":true}"[..]).unwrap(), super::DecisionTree::Solved);
    let escaped = br#"{"guess":"\u0061pple","solved":true}"#;
    assert_eq!(tree, super::DecisionTree::read_json(&escaped[..]).unwrap());
}

#[test]
fn rejects_invalid_saved_tree_nodes() {
    for text in [
        "", "null", "[]", "{}", r#"{"solved":false}"#,
        r#"{"guess":"four","solved":true}"#,
        r#"{"guess":"apple","solved":true,"branches":{}}"#,
        r#"{"guess":"apple","branches":{}}"#,
        r#"{"guess":"apple","branches":{"abcde":{"solved":true}}}"#,
        r#"{"guess":"apple","branches":{"****":{"solved":true}}}"#,
        r#"{"guess":"apple","branches":{"_____":{"solved":true}}}"#,
        r#"{"guess":"apple","branches":{"_____":{"guess":"apple","solved":true}}}"#,
        r#"{"guess":"apple","branches":{"*****":{"guess":"ample","solved":true}}}"#,
        r#"{"solved":true,"unknown":1}"#, r#"{"solved":true} trailing"#,
    ] {
        assert_eq!(super::DecisionTree::read_json(text.as_bytes()).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData, "{text}");
    }
}

#[test]
fn load_tree_cli_accepts_output_and_rejects_search_options() {
    let args = super::Args::try_parse_from(["wordlelayheehoo", "--load-tree", "tree.json", "-o", "copy.json"]).unwrap();
    assert_eq!(args.load_tree.unwrap(), std::path::PathBuf::from("tree.json"));
    for extra in [vec!["-w", "2"], vec!["--sample-percent", "10"]] {
        assert!(super::Args::try_parse_from(
            ["wordlelayheehoo", "--load-tree", "tree.json"].into_iter().chain(extra)
        ).is_err());
    }
    assert!(super::Args::try_parse_from(["wordlelayheehoo", "--load-tree"]).is_err());
}

#[test]
fn feedback_round_trips_all_letter_patterns() {
    let mut patterns = std::collections::HashSet::new();
    for a in [Grey, Gold, Green] {
        for b in [Grey, Gold, Green] {
            for c in [Grey, Gold, Green] {
                for d in [Grey, Gold, Green] {
                    for e in [Grey, Gold, Green] {
                        let letters = [a, b, c, d, e];
                        let feedback = Result::new(letters);
                        assert_eq!(feedback.to_letters(), letters);
                        assert!(patterns.insert(feedback));
                    }
                }
            }
        }
    }
    assert_eq!(patterns.len(), Result::COUNT);
    let indices: std::collections::HashSet<_> = patterns.iter().map(|r| r.index()).collect();
    assert_eq!(indices, (0..Result::COUNT).collect());
    assert_eq!(Result::ALL_GREEN.to_letters(), [Green; 5]);
    assert_eq!(Result::new([Green; 5]), Result::ALL_GREEN);
}

#[test]
fn feedback_occupies_one_byte() {
    assert_eq!(std::mem::size_of::<Result>(), 1);
    assert_eq!(std::mem::size_of::<[Result; 243]>(), 243);
}

#[test]
fn decision_tree_solves_test_candidates_after_an_opening() {
    let words: Vec<_> = TEST_WORDS.iter().map(|word| Word::new(word).unwrap()).collect();
    let target = Word::new("apple").unwrap();
    let opening = [Word::new("arose").unwrap(), Word::new("unlit").unwrap()];
    let candidates: Vec<_> = words.iter().copied().filter(|&candidate| {
        opening.iter().all(|&guess| {
            Result::from_guess(guess, candidate) == Result::from_guess(guess, target)
        })
    }).collect();
    let allowed: Vec<_> = words.iter().copied()
        .filter(|word| !opening.contains(word)).collect();
    assert!(candidates.contains(&target));
    assert!(candidates.len() > 1);
    let budget = MAX_GUESSES - opening.len();
    let tree = super::solver::MinimaxSolver::new(10)
        .decision_tree(&allowed, &candidates, budget).unwrap();
    assert!((2..=budget).contains(&tree.worst_case_guesses()));
    verify_decision_tree(&tree, &allowed, &candidates, &opening.into(), budget);
}
