use crate::common;

#[test]
fn matches_cpython_non_final_branch_while_break_locations() {
    for outer in ["for group in groups:", "while ready():"] {
        for condition in ["index < limit", "check(index)", "flag"] {
            for update in ["index += 1", "index = update(index)"] {
                let source = format!(
                    "def f(groups, flag, limit):\n    {outer}\n        if flag:\n            index = 0\n            while {condition}:\n                consume(index)\n                if done(index):\n                    break\n                {update}\n        else:\n            consume(flag)\n    return\n"
                );
                common::assert_matches_cpython_marshal(&source, "branch_while_break_locations.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_tail_while_breaks() {
    for prefix in [
        "",
        "        consume(group)\n",
        "        if skip(group):\n            continue\n",
    ] {
        for condition in ["index < len(group)", "check(index)", "flag"] {
            for update in ["index += 1", "index = update(index)"] {
                for after in ["", "        consume(index)\n"] {
                    let source = format!(
                        "def f(groups, flag):\n    result = []\n    for group in groups:\n{prefix}        index = 0\n        while {condition}:\n            consume(index)\n            if done(index):\n                break\n            {update}\n{after}    return result\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_tail_while_breaks.py");
                }
            }
        }
    }
}
