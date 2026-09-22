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

#[test]
fn matches_cpython_mapping_wildcard_capture_owner_boundaries() {
    for source in [
        "def f(subject):\n    try:\n        match subject:\n            case {'a': x, 'b': y, 'd': _}:\n                consume(x, y)\n    except Exception:\n        recover()\n",
        "def f(subject, manager):\n    with manager:\n        match subject:\n            case {'a': x, 'b': y, 'd': _}:\n                consume(x, y)\n",
        "def f(subject):\n    global x, y\n    match subject:\n        case {'a': x, 'b': y, 'd': _}:\n            consume(x, y)\n",
    ] {
        common::assert_matches_cpython_marshal(source, "mapping_wildcard_owners.py");
    }
}
