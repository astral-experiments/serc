use crate::common;

#[test]
fn folded_populated_predecessors_keep_original_tuple_copy_rounds() {
    for value in [
        "1",
        "-1",
        "(1, 2, 3)",
        "values[None, :]",
        "prepare(values, (0, 1), mode='constant', constant_values=0)",
    ] {
        for returned in ["left, right", "left, right, values"] {
            let source = format!(
                "def choose(outer, first, second, left, right, values):
    if outer:
        if first:
            left = {value}
        elif second:
            if left:
                right = False
            else:
                right = True
        else:
            raise ValueError
    else:
        left = 4
    return {returned}
"
            );
            common::assert_matches_cpython_marshal(&source, "folded_predecessor_tuple_copy.py");
        }
    }
}

#[test]
fn iterator_break_join_keeps_its_line_after_nested_return_copy() {
    let source = r"def choose(items, enabled, alternate):
    if enabled:
        if items:
            for item in items:
                inspect(item)
                if alternate:
                    break
        elif alternate:
            raise ValueError()
    elif alternate:
        raise ValueError()
    return items
";
    for exit in ["return items", "return 1", "return", "return finish(items)"] {
        for loop_exit in ["break", "inspect(item)"] {
            let source = source
                .replace("return items", exit)
                .replace("break", loop_exit);
            common::assert_matches_cpython_marshal(&source, "iterator_break_nested_return.py");
        }
    }
}

#[test]
fn nested_raising_finally_keeps_unique_condition_exit_location() {
    for (declaration, prefix, suffix) in [
        ("async def", "    await work()\n", "await finish()"),
        ("def", "", "finish()"),
        ("def", "    yield None\n", "yield sock"),
    ] {
        for cleanup in ["exceptions = None", "cleanup(exceptions)"] {
            let source = format!(
                "{declaration} f(host, sock, exceptions, all_errors):
    capture = lambda: exceptions
{prefix}    if host:
        if sock is None:
            exceptions = [exc for sub in exceptions for exc in sub]
            try:
                if all_errors:
                    raise Group(exceptions)
                raise Error(exceptions)
            finally:
                {cleanup}
    else:
        consume(sock)
    {suffix}
"
            );
            common::assert_matches_cpython_marshal(&source, "nested_raising_finally_exit.py");
        }
    }
}
