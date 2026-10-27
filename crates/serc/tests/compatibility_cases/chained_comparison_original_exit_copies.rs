use crate::common;

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
fn chained_comparison_try_exits_inherit_their_predecessors_positions() {
    for handler in [
        "except Error as err:\n    report(err)\nelse:\n    raise Other()",
        "except Error:\n    report()",
        "except* Error as err:\n    report(err)",
        "finally:\n    cleanup()",
    ] {
        let handler = handler.replace('\n', "\n        ");
        let source = format!(
            "def f(value):\n    if 0 < value < 10:\n        try:\n            work()\n        {handler}\n"
        );
        common::assert_matches_cpython_marshal(&source, "chained_comparison_try_exits.py");
    }
}
