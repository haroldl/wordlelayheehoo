use super::Word;

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
