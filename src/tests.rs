use clap::Parser;
use super::{GameState, LetterResult, MAX_GUESSES, Result, Word, WORDS, possible_states_after_guesses};
use LetterResult::{Gold, Green, Grey};

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
    assert_eq!(result.as_slice(), &expected, "guess {guess}, target {target}");
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
fn game_state_equality_ignores_guess_order() {
    let target = Word::new("apple").unwrap();
    let first = Word::new("arose").unwrap();
    let second = Word::new("unlit").unwrap();
    let first_result = Result::from_guess(first, target);
    let second_result = Result::from_guess(second, target);
    let mut forward = GameState::new();
    let mut reverse = GameState::new();

    forward = forward.with_guess(first, first_result).unwrap();
    forward = forward.with_guess(second, second_result).unwrap();
    reverse = reverse.with_guess(second, second_result).unwrap();
    reverse = reverse.with_guess(first, first_result).unwrap();

    assert_eq!(forward, reverse);
    assert_eq!(forward.guesses().len(), 2);
    assert!(forward.guesses().contains(&(first, first_result)));
    assert!(forward.guesses().contains(&(second, second_result)));
}

#[test]
fn game_state_rejects_duplicate_words_regardless_of_feedback() {
    let word = Word::new("apple").unwrap();
    let result = Result::from_guess(word, word);
    let empty = GameState::new();
    let state = empty.with_guess(word, result).unwrap();
    let duplicate = state.with_guess(word, result);

    assert!(empty.guesses().is_empty());
    assert_eq!(duplicate, None);
    assert_eq!(state.with_guess(word, Result::new([Grey; 5])), None);
    assert!(state.has_guessed(Word::new("APPLE").unwrap()));
    assert_eq!(state.with_guess(Word::new("APPLE").unwrap(), result), None);
    assert_eq!(state.guesses().len(), 1);
}

#[test]
fn game_state_equality_includes_feedback() {
    let word = Word::new("apple").unwrap();
    let mut green_state = GameState::new();
    let mut grey_state = GameState::new();
    green_state = green_state.with_guess(word, Result::new([Green; 5])).unwrap();
    grey_state = grey_state.with_guess(word, Result::new([Grey; 5])).unwrap();

    assert_ne!(green_state, grey_state);
}

#[test]
fn empty_state_allows_every_dictionary_word_in_order() {
    let state = GameState::new();
    assert!(state.possible_solutions().eq(WORDS.iter().copied()));
    assert_eq!(state.possible_solutions().next(), WORDS.first().copied());
}

#[test]
fn all_green_feedback_leaves_only_the_guessed_word() {
    let word = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(word, Result::new([Green; 5])).unwrap();
    assert_eq!(state.possible_solutions().collect::<Vec<_>>(), vec![word]);
}

#[test]
fn possible_solutions_respect_duplicate_letters_and_additional_guesses() {
    let apple = Word::new("apple").unwrap();
    let ample = Word::new("ample").unwrap();
    let allee = Word::new("allee").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(allee, Result::new([Green, Gold, Grey, Grey, Green])).unwrap();

    let initial: Vec<_> = state.possible_solutions().collect();
    assert!(initial.contains(&apple));
    assert!(initial.contains(&ample));
    assert!(!initial.contains(&allee));
    assert!(!initial.contains(&Word::new("alley").unwrap()));

    state = state.with_guess(ample, Result::new([Green, Grey, Green, Green, Green])).unwrap();
    let narrowed: Vec<_> = state.possible_solutions().collect();
    assert!(narrowed.contains(&apple));
    assert!(!narrowed.contains(&ample));
    assert!(narrowed.iter().all(|word| initial.contains(word)));
}

#[test]
fn contradictory_feedback_has_no_possible_solution() {
    let word = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(word, Result::new([Green; 5])).unwrap();
    state = state.with_guess(Word::new("ample").unwrap(), Result::new([Grey; 5])).unwrap();
    assert_eq!(state.possible_solutions().next(), None);
}

#[test]
fn state_sets_deduplicate_regardless_of_guess_order() {
    let first = Word::new("arose").unwrap();
    let second = Word::new("unlit").unwrap();
    let forward = possible_states_after_guesses(&[first, second]);
    let reverse = possible_states_after_guesses(&[second, first]);
    let repeated = possible_states_after_guesses(&[first, second, first]);
    assert_eq!(forward, reverse);
    assert!(repeated.is_empty());
    assert!(forward.len() < WORDS.len());
    assert!(forward.iter().all(|state| state.guesses().len() == 2));
}

#[test]
fn generated_states_cover_every_target() {
    let guesses = [Word::new("arose").unwrap(), Word::new("unlit").unwrap()];
    let states = possible_states_after_guesses(&guesses);
    let mut witnessed = std::collections::HashSet::new();
    for &target in WORDS {
        let mut expected = GameState::new();
        for guess in guesses {
            expected = expected.with_guess(guess, Result::from_guess(guess, target)).unwrap();
        }
        assert!(states.contains(&expected), "missing target {}", target.as_str());
        witnessed.insert(expected);
    }
    // Every generated state has at least one dictionary target as a witness.
    assert_eq!(states.len(), witnessed.len());
}

#[test]
fn no_guesses_produce_one_empty_state() {
    let states = possible_states_after_guesses(&[]);
    assert_eq!(states.len(), 1);
    assert!(states.contains(&GameState::new()));
}

#[test]
fn empty_game_is_not_solved() {
    assert!(!GameState::new().is_solved());
}

#[test]
fn partial_matches_do_not_solve_the_game() {
    let guess = Word::new("apple").unwrap();
    for non_green in [Grey, Gold] {
        for position in 0..5 {
            let mut feedback = [Green; 5];
            feedback[position] = non_green;
            let mut state = GameState::new();
            state = state.with_guess(guess, Result::new(feedback)).unwrap();
            assert!(!state.is_solved());
        }
    }
}

#[test]
fn correct_guess_solves_game_regardless_of_other_guesses() {
    let target = Word::new("apple").unwrap();
    let miss = Word::new("arose").unwrap();
    for guesses in [[miss, target], [target, miss]] {
        let mut state = GameState::new();
        for guess in guesses {
            state = state.with_guess(guess, Result::from_guess(guess, target)).unwrap();
        }
        assert!(state.is_solved());
    }
}

#[test]
fn make_guess_from_empty_state_matches_all_dictionary_outcomes() {
    let guess = Word::new("arose").unwrap();
    let state = GameState::new();
    assert_eq!(state.make_guess(guess), possible_states_after_guesses(&[guess]));
    assert!(state.guesses().is_empty());
}

#[test]
fn make_guess_preserves_history_and_only_uses_current_candidates() {
    let target = Word::new("apple").unwrap();
    let first = Word::new("allee").unwrap();
    let guess = Word::new("ample").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(first, Result::from_guess(first, target)).unwrap();
    let original = state.clone();
    let next_states = state.make_guess(guess);
    assert_eq!(state, original);
    assert!(next_states.len() > 1);

    for next in &next_states {
        assert!(state.guesses().is_subset(next.guesses()));
        assert_eq!(next.guesses().len(), 2);
        assert!(next.possible_solutions().next().is_some());
    }
    // The next states partition the current candidates, with no targets lost
    // or assigned to multiple feedback outcomes.
    let mut partition = Vec::new();
    for next in &next_states {
        partition.extend(next.possible_solutions());
    }
    partition.sort_unstable();
    let mut expected: Vec<_> = state.possible_solutions().collect();
    expected.sort_unstable();
    assert_eq!(partition, expected);
}

#[test]
fn repeating_a_guess_has_no_successors() {
    let guess = Word::new("arose").unwrap();
    let target = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(guess, Result::from_guess(guess, target)).unwrap();
    let next = state.make_guess(guess);
    assert!(next.is_empty());
    assert_eq!(state.guesses().len(), 1);
}

#[test]
fn make_guess_on_contradictory_state_has_no_outcomes() {
    let word = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(word, Result::new([Green; 5])).unwrap();
    state = state.with_guess(Word::new("ample").unwrap(), Result::new([Grey; 5])).unwrap();
    assert!(state.make_guess(Word::new("arose").unwrap()).is_empty());
}

#[test]
fn make_guess_after_a_win_scores_against_the_known_target() {
    let target = Word::new("apple").unwrap();
    let guess = Word::new("arose").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(target, Result::new([Green; 5])).unwrap();
    let next = state.make_guess(guess);
    let mut expected = state.clone();
    expected = expected.with_guess(guess, Result::from_guess(guess, target)).unwrap();
    assert_eq!(next.len(), 1);
    assert!(next.contains(&expected));
}

#[test]
fn adding_feedback_preserves_states_already_in_a_hash_set() {
    let target = Word::new("apple").unwrap();
    let first = Word::new("arose").unwrap();
    let original = GameState::new().with_guess(first, Result::from_guess(first, target)).unwrap();
    let mut states = std::collections::HashSet::from([original.clone()]);
    let stored = states.get(&original).unwrap();
    let extended = stored.with_guess(target, Result::new([Green; 5])).unwrap();

    assert_eq!(original.guesses().len(), 1);
    assert!(!original.is_solved());
    assert_eq!(extended.guesses().len(), 2);
    assert!(extended.is_solved());
    assert!(states.contains(&original));
    assert!(!states.contains(&extended));
    assert!(states.insert(extended));
    assert_eq!(states.len(), 2);
}

#[test]
fn game_is_lost_at_or_beyond_the_guess_limit() {
    let target = Word::new("apple").unwrap();
    let mut state = GameState::new();
    assert!(!state.is_game_lost());
    for (index, guess) in WORDS.iter().copied().filter(|&word| word != target)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter().take(MAX_GUESSES + 1).enumerate()
    {
        state = state.with_guess(guess, Result::from_guess(guess, target)).unwrap();
        assert_eq!(state.is_game_lost(), index + 1 >= MAX_GUESSES);
    }
    assert_eq!(state.guesses().len(), MAX_GUESSES + 1);
}

#[test]
fn solved_game_is_not_lost_at_or_beyond_the_guess_limit() {
    let target = Word::new("apple").unwrap();
    let mut state = GameState::new().with_guess(target, Result::new([Green; 5])).unwrap();
    for guess in WORDS.iter().copied().filter(|&word| word != target)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter().take(MAX_GUESSES)
    {
        state = state.with_guess(guess, Result::from_guess(guess, target)).unwrap();
        assert!(!state.is_game_lost());
    }
    assert_eq!(state.guesses().len(), MAX_GUESSES + 1);
}

#[test]
fn minimax_counts_the_final_guess_and_respects_budget() {
    let words: Vec<_> = ["aaaaa", "baaaa", "caaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    assert_eq!(super::solver::minimum_guesses(&words, &words[..1], 1), Some(1));
    assert_eq!(super::solver::minimum_guesses(&words, &words[..1], 0), None);
    assert_eq!(super::solver::minimum_guesses(&words, &words, 2), None);
    assert_eq!(super::solver::minimum_guesses(&words, &words, 3), Some(3));
    assert_eq!(super::solver::minimum_guesses(&words, &[], 3), None);
}

#[test]
fn minimax_can_use_a_non_candidate_probe() {
    let candidates: Vec<_> = ["aaaaa", "baaaa", "caaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    let mut allowed = candidates.clone();
    allowed.push(Word::new("bcddd").unwrap());
    // bcddd produces three distinct non-winning results, then the answer
    // must be guessed. Guessing only candidates takes three in the worst case.
    assert_eq!(super::solver::minimum_guesses(&allowed, &candidates, 2), Some(2));
    allowed.push(allowed[0]);
    let mut repeated = candidates.clone();
    repeated.push(candidates[0]);
    assert_eq!(super::solver::minimum_guesses(&allowed, &repeated, 2), Some(2));
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
            super::solver::minimum_guesses(&allowed, &candidates, expected),
            Some(expected), "subset {mask}"
        );
        assert_eq!(
            super::solver::minimum_guesses(&allowed, &candidates, expected - 1),
            None, "subset {mask} fits below its optimum"
        );
    }
}

#[test]
fn minimax_game_state_handles_solved_inconsistent_and_exhausted_states() {
    let target = Word::new("apple").unwrap();
    let solved = GameState::new().with_guess(target, Result::new([Green; 5])).unwrap();
    assert_eq!(solved.minimax_guesses(), Some(0));
    let inconsistent = solved.with_guess(
        Word::new("ample").unwrap(), Result::new([Grey; 5])
    ).unwrap();
    assert_eq!(inconsistent.minimax_guesses(), None);

    let exhausted = WORDS.iter().copied().filter(|&word| word != target)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter().take(MAX_GUESSES)
        .fold(GameState::new(), |state, guess| {
            state.with_guess(guess, Result::from_guess(guess, target)).unwrap()
        });
    assert_eq!(exhausted.minimax_guesses(), None);
}

#[test]
fn minimax_solves_a_state_after_the_development_opening() {
    let target = Word::new("apple").unwrap();
    let state = ["arose", "unlit"].into_iter().fold(GameState::new(), |state, text| {
        let guess = Word::new(text).unwrap();
        state.with_guess(guess, Result::from_guess(guess, target)).unwrap()
    });
    let candidates: Vec<_> = state.possible_solutions().collect();
    assert!(candidates.contains(&target));
    let allowed: Vec<_> = WORDS.iter().copied().filter(|&word| !state.has_guessed(word)).collect();
    assert!(candidates.len() > 1);
    // A separating guess is a two-turn certificate: each result identifies
    // one target, which can then be guessed. Multiple targets rule out one turn.
    let has_separating_guess = allowed.iter().any(|&guess| {
        let patterns: std::collections::HashSet<_> = candidates.iter()
            .map(|&target| Result::from_guess(guess, target)).collect();
        patterns.len() == candidates.len()
    });
    assert!(has_separating_guess);
    assert_eq!(state.minimax_guesses(), Some(2));
}

#[test]
fn minimax_searches_multiple_levels_when_guesses_only_eliminate_one_target() {
    let words: Vec<_> = ["aaaaa", "baaaa", "caaaa", "daaaa", "eaaaa"]
        .into_iter().map(|word| Word::new(word).unwrap()).collect();
    // Every miss leaves all other targets indistinguishable, so all five
    // guesses can be necessary. Depth four exercises recursive failed branches.
    assert_eq!(super::solver::minimum_guesses(&words, &words, 4), None);
    assert_eq!(super::solver::minimum_guesses(&words, &words, 5), Some(5));
}

#[test]
fn parallel_minimax_matches_serial_maximum_and_preserves_failure() {
    let target = Word::new("apple").unwrap();
    let solved = GameState::new().with_guess(target, Result::new([Green; 5])).unwrap();
    let opening = ["arose", "unlit"].into_iter().fold(GameState::new(), |state, text| {
        let guess = Word::new(text).unwrap();
        state.with_guess(guess, Result::from_guess(guess, target)).unwrap()
    });
    let mut states = std::collections::HashSet::from([solved.clone(), opening]);
    let serial = states.iter().map(GameState::minimax_guesses)
        .try_fold(0usize, |worst, next| next.map(|count| worst.max(count)));
    assert_eq!(serial, Some(2));
    assert_eq!(super::max_minimax_guesses(&states, 10), serial);

    let impossible = solved.with_guess(Word::new("ample").unwrap(), Result::new([Grey; 5])).unwrap();
    states.insert(impossible);
    assert_eq!(super::max_minimax_guesses(&states, 10), None);
    assert_eq!(super::max_minimax_guesses(&std::collections::HashSet::new(), 10), Some(0));
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
        let solver = super::solver::MinimaxSolver::new(workers);
        for mask in 1..(1 << allowed.len()) {
            let candidates: Vec<_> = allowed.iter().enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, &word)| word).collect();
            let expected = exhaustive_minimax(&allowed, &candidates);
            // Reuse a pool after both success (with cancellation) and failure.
            assert_eq!(solver.minimum_guesses(&allowed, &candidates, expected), Some(expected));
            assert_eq!(solver.minimum_guesses(&allowed, &candidates, expected - 1), None);
        }
    }
}

#[test]
fn queued_minimax_exhausts_multiple_batches_without_a_winning_strategy() {
    // A shared suffix makes each missed candidate eliminate only itself.
    // More than 16 words forces work into multiple queue jobs.
    let words: Vec<_> = (b'a'..=b't').map(|letter| Word::new(&format!("{}zzzz", char::from(letter))).unwrap()).collect();
    for workers in [1, 10, 20] {
        let solver = super::solver::MinimaxSolver::new(workers);
        assert_eq!(solver.minimum_guesses(&words, &words, 3), None);
        assert_eq!(solver.minimum_guesses(&words, &words[..1], 1), Some(1));
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
        if result == Result::new([Green; 5]) {
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
        let solver = super::solver::MinimaxSolver::new(workers);
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
    let solver = super::solver::MinimaxSolver::new(10);
    let tree = solver.decision_tree(&allowed, &candidates, 2).unwrap();
    assert_eq!(tree.guess(), Some(probe));
    assert_eq!(tree.after_result(Result::new([Green; 5])), None);
    verify_decision_tree(&tree, &allowed, &candidates, &std::collections::HashSet::new(), 2);
}

#[test]
fn decision_tree_for_game_state_preserves_history_and_budget() {
    let target = Word::new("apple").unwrap();
    let state = ["arose", "unlit"].into_iter().fold(GameState::new(), |state, text| {
        let guess = Word::new(text).unwrap();
        state.with_guess(guess, Result::from_guess(guess, target)).unwrap()
    });
    let tree = state.decision_tree_with_workers(10).unwrap();
    assert_eq!(tree.worst_case_guesses(), 2);
    let played = state.guesses().iter().map(|&(word, _)| word).collect();
    let candidates: Vec<_> = state.possible_solutions().collect();
    verify_decision_tree(&tree, WORDS, &candidates, &played, MAX_GUESSES - state.guesses().len());
    let solved = state.with_guess(target, Result::new([Green; 5])).unwrap();
    assert_eq!(solved.decision_tree(), Some(super::DecisionTree::Solved));
    let inconsistent = solved.with_guess(Word::new("ample").unwrap(), Result::new([Grey; 5])).unwrap();
    assert_eq!(inconsistent.decision_tree(), None);
}
