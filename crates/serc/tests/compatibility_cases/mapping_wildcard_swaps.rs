use crate::common;

#[test]
fn matches_cpython_mapping_wildcard_capture_rotations() {
    for (captures, wildcards) in [(1, 1), (1, 2), (2, 1), (2, 2), (3, 1), (4, 1)] {
        let entries = (0..captures)
            .map(|index| format!("'k{index}': v{index}"))
            .chain((0..wildcards).map(|index| format!("'w{index}': _")))
            .collect::<Vec<_>>()
            .join(", ");
        common::assert_matches_cpython_marshal(
            &format!(
                "def f(subject):\n    match subject:\n        case {{{entries}}}:\n            consume()\n    return None\n"
            ),
            "mapping_wildcard_rotations.py",
        );
    }
}
