use crate::common;

#[test]
fn matches_cpython_tuple_store_join_locations() {
    for (prefix, assignment) in [
        ("", "left, right = [], []"),
        ("", "left, right = first(), second()"),
        ("", "left, right = 1, 2"),
        ("", "left, right, spare = [], [], []"),
        // These stores cannot be exchanged by the static SWAP pass.
        ("", "left, left = [], []"),
        ("    global left, right\n", "left, right = [], []"),
        (
            "    def capture():\n        return left\n",
            "left, right = [], []",
        ),
        ("", "(left,\n             right) = [], []"),
    ] {
        let source = format!(
            "def f(a, b):\n{prefix}    if a:\n        if b:\n            left = work()\n            right = work()\n        else:\n            {assignment}\n    else:\n        {assignment}\n    return left, right\n"
        );
        common::assert_matches_cpython_marshal(&source, "tuple_store_join.py");
    }
}
