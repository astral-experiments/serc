use crate::common;

#[test]
fn preserves_direct_nested_pass_return_borrow_boundaries() {
    for otherwise in ["pass", "value = update(other)"] {
        for returned in ["value", "42"] {
            let source = format!(
                "def f(value, other, outer, first, second):
    if outer:
        if first:
            value = update(value)
        elif second:
            value = update(other)
        else:
            {otherwise}
    else:
        value = update(other)
    return {returned}
"
            );
            common::assert_matches_cpython_marshal(&source, "direct_nested_pass_return.py");
        }
    }
}

#[test]
fn borrows_threaded_returns_after_populated_conditional_arrays() {
    for leading in [
        "",
        "            candidate = check(items)\n            if candidate is None:\n                return []\n",
    ] {
        for condition in ["check(items)", "check(items) and check(node)"] {
            for populated in [
                "                    nodes.append(check(items))\n",
                "                    for item in items:\n                        found = check(item)\n                        if found:\n                            nodes.append(found)\n",
            ] {
                let source = format!(
                    "def function(node, items, flag, check):\n    if node is None:\n        if flag:\n{leading}            nodes = []\n            if not nodes:\n                if {condition}:\n{populated}                else:\n                    return []\n        else:\n            return []\n    else:\n        nodes = [node]\n    return nodes\n",
                );
                common::assert_matches_cpython_marshal(&source, "threaded_conditional_return.py");
            }
        }
    }
}
