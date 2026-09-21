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

#[test]
fn matches_cpython_tuple_store_shared_and_flat_join_controls() {
    for source in [
        // The inner join becomes private, while the enclosing join remains shared.
        r"def f(a, b, c, d):
    if a:
        left = work()
        right = work()
    elif b:
        if c:
            if d:
                left = work()
                right = work()
            else:
                left, right = [], []
        else:
            left, right = [], []
    else:
        left, right = [], []
    return left, right
",
        r"def f(a):
    if a:
        left, right = [], []
    else:
        left, right = [], []
    return left, right
",
    ] {
        common::assert_matches_cpython_marshal(source, "tuple_store_shared_join.py");
    }
}
