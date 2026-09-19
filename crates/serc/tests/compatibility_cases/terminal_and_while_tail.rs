use crate::common;

#[test]
fn normalizes_tail_conditionals_in_terminal_and_while_loops() {
    for definition in ["def", "async def"] {
        for condition in ["a and b", "a and b and c"] {
            for body in [
                "        if flag:\n            pass\n",
                "        if not flag:\n            use()\n",
                "        if flag:\n            a = flag\n",
                "        if flag:\n            return\n",
            ] {
                let source =
                    format!("{definition} f(a, b, c, flag):\n    while {condition}:\n{body}");
                common::assert_matches_cpython_marshal(&source, "terminal_and_while_tail.py");
            }
        }
    }
}

#[test]
fn preserves_folded_selectors_in_terminal_and_while_loops() {
    for (definition, prefix) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for selector in ["True", "1", "1.0", "1j", "'yes'", "b'yes'"] {
            let source = format!(
                "{definition} f(a, b, flag):\n{prefix}    while a and b:\n        if {selector}:\n            use(flag)\n"
            );
            common::assert_matches_cpython_marshal(&source, "terminal_and_while_folded_tail.py");
        }
    }
}

#[test]
fn duplicates_implicit_return_for_mixed_boolean_terminal_while() {
    common::assert_matches_cpython_marshal(
        r"async def f(state, last=False):
    while state.count >= state.limit or (last and state.count > 0):
        await state.stage()
        state.count = state.count - state.limit
",
        "terminal_mixed_boolean_while_async.py",
    );
}

#[test]
fn normalizes_complete_terminal_while_branches_without_final_fallthrough() {
    common::assert_matches_cpython_marshal(
        r"def probe(fetch):
    while True:
        result = fetch()
        if isinstance(result, dict):
            for item in result.get('items', []):
                yield item
            marker = result.get('next')
            if not marker:
                break
        else:
            for item in result:
                yield item
            break
",
        "generator_nested_loop_exits.py",
    );
}
