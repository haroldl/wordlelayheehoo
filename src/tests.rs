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

    forward = forward.with_guess(first, first_result);
    forward = forward.with_guess(second, second_result);
    reverse = reverse.with_guess(second, second_result);
    reverse = reverse.with_guess(first, first_result);

    assert_eq!(forward, reverse);
    assert_eq!(forward.guesses().len(), 2);
    assert!(forward.guesses().contains(&(first, first_result)));
    assert!(forward.guesses().contains(&(second, second_result)));
}

#[test]
fn game_state_ignores_duplicate_pairs() {
    let word = Word::new("apple").unwrap();
    let result = Result::from_guess(word, word);
    let empty = GameState::new();
    let state = empty.with_guess(word, result);
    let duplicate = state.with_guess(word, result);

    assert!(empty.guesses().is_empty());
    assert_eq!(duplicate, state);
    assert_eq!(state.guesses().len(), 1);
}

#[test]
fn game_state_equality_includes_feedback() {
    let word = Word::new("apple").unwrap();
    let mut green_state = GameState::new();
    let mut grey_state = GameState::new();
    green_state = green_state.with_guess(word, Result::new([Green; 5]));
    grey_state = grey_state.with_guess(word, Result::new([Grey; 5]));

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
    state = state.with_guess(word, Result::new([Green; 5]));
    assert_eq!(state.possible_solutions().collect::<Vec<_>>(), vec![word]);
}

#[test]
fn possible_solutions_respect_duplicate_letters_and_additional_guesses() {
    let apple = Word::new("apple").unwrap();
    let ample = Word::new("ample").unwrap();
    let allee = Word::new("allee").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(allee, Result::new([Green, Gold, Grey, Grey, Green]));

    let initial: Vec<_> = state.possible_solutions().collect();
    assert!(initial.contains(&apple));
    assert!(initial.contains(&ample));
    assert!(!initial.contains(&allee));
    assert!(!initial.contains(&Word::new("alley").unwrap()));

    state = state.with_guess(ample, Result::new([Green, Grey, Green, Green, Green]));
    let narrowed: Vec<_> = state.possible_solutions().collect();
    assert!(narrowed.contains(&apple));
    assert!(!narrowed.contains(&ample));
    assert!(narrowed.iter().all(|word| initial.contains(word)));
}

#[test]
fn contradictory_feedback_has_no_possible_solution() {
    let word = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(word, Result::new([Green; 5]));
    state = state.with_guess(word, Result::new([Grey; 5]));
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
    assert_eq!(forward, repeated);
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
            expected = expected.with_guess(guess, Result::from_guess(guess, target));
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
            state = state.with_guess(guess, Result::new(feedback));
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
            state = state.with_guess(guess, Result::from_guess(guess, target));
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
    state = state.with_guess(first, Result::from_guess(first, target));
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
fn repeating_a_guess_returns_the_same_state() {
    let guess = Word::new("arose").unwrap();
    let target = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(guess, Result::from_guess(guess, target));
    let next = state.make_guess(guess);
    assert_eq!(next.len(), 1);
    assert!(next.contains(&state));
}

#[test]
fn make_guess_on_contradictory_state_has_no_outcomes() {
    let word = Word::new("apple").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(word, Result::new([Green; 5]));
    state = state.with_guess(word, Result::new([Grey; 5]));
    assert!(state.make_guess(Word::new("arose").unwrap()).is_empty());
}

#[test]
fn make_guess_after_a_win_scores_against_the_known_target() {
    let target = Word::new("apple").unwrap();
    let guess = Word::new("arose").unwrap();
    let mut state = GameState::new();
    state = state.with_guess(target, Result::new([Green; 5]));
    let next = state.make_guess(guess);
    let mut expected = state.clone();
    expected = expected.with_guess(guess, Result::from_guess(guess, target));
    assert_eq!(next.len(), 1);
    assert!(next.contains(&expected));
}

#[test]
fn adding_feedback_preserves_states_already_in_a_hash_set() {
    let target = Word::new("apple").unwrap();
    let first = Word::new("arose").unwrap();
    let original = GameState::new().with_guess(first, Result::from_guess(first, target));
    let mut states = std::collections::HashSet::from([original.clone()]);
    let stored = states.get(&original).unwrap();
    let extended = stored.with_guess(target, Result::new([Green; 5]));

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
        state = state.with_guess(guess, Result::from_guess(guess, target));
        assert_eq!(state.is_game_lost(), index + 1 >= MAX_GUESSES);
    }
    assert_eq!(state.guesses().len(), MAX_GUESSES + 1);
}

#[test]
fn solved_game_is_not_lost_at_or_beyond_the_guess_limit() {
    let target = Word::new("apple").unwrap();
    let mut state = GameState::new().with_guess(target, Result::new([Green; 5]));
    for guess in WORDS.iter().copied().filter(|&word| word != target)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter().take(MAX_GUESSES)
    {
        state = state.with_guess(guess, Result::from_guess(guess, target));
        assert!(!state.is_game_lost());
    }
    assert_eq!(state.guesses().len(), MAX_GUESSES + 1);
}
