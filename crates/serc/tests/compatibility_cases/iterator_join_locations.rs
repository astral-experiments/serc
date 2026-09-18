use crate::common;

#[test]
fn inherits_threaded_and_unique_false_join_locations() {
    for (filename, source) in [
        (
            "iterator_join_protected_nested_if.py",
            r"def convert(items, selected, ready, output, state):
    skipped = 0
    last = None
    for item in items:
        if selected(item):
            try:
                if item.kind == 0:
                    output.append(item)
                    if ready(item):
                        state.append(item)
                        last = 0
                elif item.kind == 1:
                    if not state:
                        return None
                    output.append(item)
                    if ready(item):
                        state.append(item)
                        last = 1
                else:
                    skipped += 1
            except Error:
                skipped += 1
        else:
            output.append(item)
    return output, last, skipped
",
        ),
        (
            "while_join_protected_nested_if_control.py",
            r"def convert(items, selected, ready, output, state):
    skipped = 0
    last = None
    while items:
        item = items.pop()
        if selected(item):
            try:
                if item.kind == 0:
                    output.append(item)
                    if ready(item):
                        state.append(item)
                        last = 0
                elif item.kind == 1:
                    if not state:
                        return None
                    output.append(item)
                    if ready(item):
                        state.append(item)
                        last = 1
                else:
                    skipped += 1
            except Error:
                skipped += 1
        else:
            output.append(item)
    return output, last, skipped
",
        ),
        (
            "for_else_continue_unique_false_join.py",
            r"def simplify(items, selected, ready, compatible, state):
    for item in items:
        if selected(item):
            value = item.value
            if ready(value):
                for child in item.children:
                    if ready(child) or compatible(value, child):
                        if compatible(child, value):
                            state[child] = value
                        else:
                            break
                    else:
                        break
                else:
                    continue
        elif item.kind == 1:
            state[item] = None
        consume(item)
    return state
",
        ),
        (
            "while_outer_for_else_continue.py",
            r"def simplify(items, selected, ready, compatible, state):
    while items:
        item = items.pop()
        if selected(item):
            value = item.value
            if ready(value):
                for child in item.children:
                    if ready(child) or compatible(value, child):
                        if compatible(child, value):
                            state[child] = value
                        else:
                            break
                    else:
                        break
                else:
                    continue
        elif item.kind == 1:
            state[item] = None
        consume(item)
    return state
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_shared_join_location_controls() {
    for (filename, source) in [
        (
            "historical_broad_promotion_seed.py",
            r"def loop_join(items, enabled, alternate, ready, begin):
    for item in items:
        if enabled:
            if ready(item):
                begin()
        elif alternate:
            if ready(item):
                break
",
        ),
        (
            "for_else_pass_shared_join_control.py",
            r"def simplify(items, selected, ready, compatible, state):
    for item in items:
        if selected(item):
            value = item.value
            if ready(value):
                for child in item.children:
                    if ready(child) or compatible(value, child):
                        if compatible(child, value):
                            state[child] = value
                        else:
                            break
                    else:
                        break
                else:
                    state[item] = value
        elif item.kind == 1:
            state[item] = None
        consume(item)
    return state
",
        ),
        (
            "for_without_else_shared_join_control.py",
            r"def simplify(items, selected, ready, compatible, state):
    for item in items:
        if selected(item):
            value = item.value
            if ready(value):
                for child in item.children:
                    if ready(child) or compatible(value, child):
                        if compatible(child, value):
                            state[child] = value
                        else:
                            break
                    else:
                        break
        elif item.kind == 1:
            state[item] = None
        consume(item)
    return state
",
        ),
        (
            "for_else_continue_multiple_false_control.py",
            r"def simplify(items, selected, ready, compatible, state):
    for item in items:
        if selected(item):
            value = item.value
            if ready(value) and selected(value):
                for child in item.children:
                    if ready(child) or compatible(value, child):
                        if compatible(child, value):
                            state[child] = value
                        else:
                            break
                    else:
                        break
                else:
                    continue
        elif item.kind == 1:
            state[item] = None
        consume(item)
    return state
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
