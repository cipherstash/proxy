pub fn assert_expected<T>(expected: &[T], actual: &[T])
where
    T: std::fmt::Display + PartialEq + std::fmt::Debug,
{
    assert_eq!(expected.len(), actual.len());
    for (e, a) in expected.iter().zip(actual) {
        assert_eq!(e, a);
    }
}
