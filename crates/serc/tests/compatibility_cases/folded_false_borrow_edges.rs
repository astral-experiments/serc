use crate::common;

#[test]
fn borrows_independently_reachable_for_cleanup_after_folded_false() {
    for (definition, action) in [("def", "emit(item)"), ("async def", "await emit(item)")] {
        let source = format!(
            "{definition} f(items, emit):\n    count = 0\n    other = 1\n    for item in items:\n        if item is None:\n            other += 1\n            continue\n        if False:\n            emit(item)\n            continue\n        {action}\n        count += 1\n    emit(count, other)\n"
        );
        common::assert_matches_cpython_marshal(&source, "folded_false_for_cleanup.py");
    }
}

#[test]
fn preserves_folded_false_fallthrough_borrow_barriers() {
    for definition in ["def", "async def"] {
        for tail in ["emit(value)", "return emit(value)", "return value"] {
            let source = format!(
                "{definition} f(value, emit):\n    if False:\n        discard()\n    {tail}\n"
            );
            common::assert_matches_cpython_marshal(&source, "folded_false_fallthrough.py");
        }
    }
}

#[test]
fn preserves_for_cleanup_without_a_folded_branch() {
    for (definition, action) in [("def", "emit(item)"), ("async def", "await emit(item)")] {
        let source = format!(
            "{definition} f(items, emit):\n    count = 0\n    other = 1\n    for item in items:\n        {action}\n        count += 1\n    emit(count, other)\n"
        );
        common::assert_matches_cpython_marshal(&source, "ordinary_for_cleanup.py");
    }
}

#[test]
fn preserves_folded_false_loop_tail_else_borrow_barriers() {
    let source = "
def f(items, first, second):
    for value in items:
        message = first + second
        if False:
            discard(value)
        else:
            assert digest(first) == digest(second), message
    return items
";
    common::assert_matches_cpython_marshal(source, "folded_false_loop_tail_else.py");
}

#[test]
fn preserves_folded_false_loop_tail_else_with_backedge() {
    let source = "
def f(items, emit):
    for item in items:
        remaining = item
        if 0:
            discard(item)
        else:
            while remaining > 0:
                emit(item, remaining)
                remaining -= 1
    return items
";
    common::assert_matches_cpython_marshal(source, "folded_false_loop_tail_else_backedge.py");
}

#[test]
fn preserves_folded_false_async_loop_tail_else_handler() {
    let source = "
async def f(stream, worker):
    async for value in stream:
        if False:
            discard(value)
        else:
            try:
                output = worker.encode(value)
                value.complete(output)
            except Exception as error:
                report(error)
    return worker
";
    common::assert_matches_cpython_marshal(source, "folded_false_async_loop_tail_else.py");
}

#[test]
fn preserves_folded_true_chain_module_comprehension_borrow_boundaries() {
    for (first, second) in [("True", "False"), ("flag", "True")] {
        let source = format!(
            "before = [value for value in values]
if {first}:
    selection = choose(values)
elif {second}:
    selection = choose(other)
elif False:
    selection = []
else:
    selection = [1] * 10
    pass
consume(selection)
for item in selection:
    if False:
        consume(other)
    consume(item)
after = [value for value, _ in pairs]
converted = [str(value) for value in after]
"
        );
        common::assert_matches_cpython_marshal(&source, "folded_true_chain_module_borrows.py");
    }
}
