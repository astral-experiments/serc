use crate::common;

#[test]
fn preserves_original_chained_comparison_exit_copies() {
    common::assert_matches_cpython_marshal(
        include_str!("../fixtures/chained_comparison_original_exit_copies.py"),
        "chained_comparison_original_exit_copies.py",
    );
}

#[test]
fn matches_cpython_small_chained_comparison_return_bridges() {
    for body in [
        "return GLOBAL.value",
        "return seq[index]",
        "return seq[index].value",
        "return (None,) * 4",
        "return (\n    obj.value\n)",
        "return (\n    seq[index]\n)",
    ] {
        let if_body = format!("        {}\n", body.replace('\n', "\n        "));
        let case_body = format!("            {}\n", body.replace('\n', "\n            "));
        for source in [
            format!("def f(seq, index, obj, value):\n    if 0 <= index < len(seq):\n{if_body}"),
            format!(
                "def f(seq, index, obj, value):\n    match value:\n        case _ as index if 0 <= index < len(seq):\n{case_body}"
            ),
            format!(
                "def f(seq, index, obj, value):\n    match value:\n        case 0:\n            return None\n        case _ if 0 <= index < len(seq):\n{case_body}"
            ),
        ] {
            common::assert_matches_cpython_marshal(&source, "chained_comparison_return_bridges.py");
        }
    }
    common::assert_matches_cpython_marshal(
        "def f(seq, index, value):\n    match value:\n        case _ as index if 0 <= index < len(seq):\n            return seq[index]\n        case 0:\n            return None\n",
        "chained_comparison_copied_subject.py",
    );
}

#[test]
fn matches_cpython_unlabeled_comparison_subscript_join_copies() {
    for source in [
        r"def probe(gate, other, a, b, c, values):
    if gate:
        if a < b < c:
            if other:
                index = 1
            else:
                index = 2
        else:
            index = 3
    else:
        index = 4
    return values[index]
",
        r"def probe(gate, other, a, b, c, values):
    if gate:
        if other:
            index = 1 if a < b < c else 2
        else:
            index = 3
    else:
        index = 4
    return values[index]
",
    ] {
        common::assert_matches_cpython_marshal(source, "comparison_subscript_join_copies.py");
    }
}
