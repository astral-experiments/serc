#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_iterator_try_assignment_branch_exits() {
    let source = r#"def nested_loop(values, enabled, conds, wrapper, fallback, finalize):
    for value in values:
        if enabled:
            try:
                if conds == "piecewise":
                    result = wrapper(value, hints=[])
                else:
                    result = fallback(value, hints=[])
            except ValueError:
                result = None
        else:
            result = None
        finalize(result)

def imported_nested_loop(values, enabled, conds, finalize):
    for value in values:
        if enabled:
            from collections import deque as wrapper, OrderedDict as fallback
            try:
                if conds == "piecewise":
                    result = wrapper(value)
                else:
                    result = fallback(value)
            except ValueError:
                result = None
        else:
            result = None
        finalize(result)

def deeply_nested(values, manual, heurisch, conds, wrapper, fallback, finalize):
    for value in values:
        if not manual:
            if heurisch is not False:
                from collections import deque
                try:
                    if conds == "piecewise":
                        result = wrapper(value, hints=[])
                    else:
                        result = fallback(value, hints=[])
                except ValueError:
                    result = None
        else:
            result = None
        finalize(result)

def preceded_try(values, manual, heurisch, conds, wrapper, fallback, finalize):
    for value in values:
        if not manual:
            try:
                previous = wrapper(value)
            except RuntimeError:
                previous = None
            if heurisch is not False:
                from collections import deque
                try:
                    if conds == "piecewise":
                        result = wrapper(value, hints=[])
                    else:
                        result = fallback(value, hints=[])
                except ValueError:
                    result = None
        else:
            result = None
        finalize(result)

def direct_loop(values, conds, wrapper, fallback, finalize):
    for value in values:
        try:
            if conds == "piecewise":
                result = wrapper(value, hints=[])
            else:
                result = fallback(value, hints=[])
        except ValueError:
            result = None
        finalize(result)

def no_loop(enabled, value, conds, wrapper, fallback, finalize):
    if enabled:
        try:
            if conds == "piecewise":
                result = wrapper(value, hints=[])
            else:
                result = fallback(value, hints=[])
        except ValueError:
            result = None
    else:
        result = None
    finalize(result)

def terminal_try_control(values, conds, wrapper, fallback):
    for value in values:
        try:
            if conds == "piecewise":
                result = wrapper(value, hints=[])
            else:
                result = fallback(value, hints=[])
        except ValueError:
            result = None
    return result
"#;
    common::assert_matches_cpython_marshal(source, "iterator_try_assignment_branch_exits.py");
}
