use crate::common;

#[test]
fn protected_last_chained_comparison() {
    common::assert_matches_cpython_marshal(
        r"def target(items, ready, limit, consume):
    for item in items:
        try:
            if ready(item) and 0 <= item < limit:
                consume(item)
        except Exception:
            pass


def existing_flat_boolean(items, ready, limit, consume):
    for item in items:
        try:
            if ready(item) and item < limit:
                consume(item)
        except Exception:
            pass
",
        "protected_last_chained_comparison.py",
    );
}

#[test]
fn protected_nested_boolean() {
    common::assert_matches_cpython_marshal(
        r"def target(items, left, middle, right, consume):
    for item in items:
        try:
            if left(item) or (middle(item) and right(item)):
                consume(item)
        except Exception:
            pass


def single_false_edge(items, left, right, consume):
    for item in items:
        try:
            if left(item) or right(item):
                consume(item)
        except Exception:
            pass
",
        "protected_nested_boolean.py",
    );
}

#[test]
fn nonfinal_inner_while() {
    common::assert_matches_cpython_marshal(
        r"def target(items, first, second, consume):
    for item in items:
        if item:
            while first(item) and second(item):
                consume(item)
        else:
            consume(None)


def single_false_edge(items, first, consume):
    for item in items:
        if item:
            while first(item):
                consume(item)
        else:
            consume(None)
",
        "nonfinal_inner_while.py",
    );
}

#[test]
fn terminal_elif() {
    common::assert_matches_cpython_marshal(
        r"def terminal_elif(read, consume):
    while True:
        value = read()
        if value == 0:
            consume(value)
        elif value == 1:
            consume(-value)
        elif value == 2:
            break


def terminal_elif_with_following(read, consume):
    while True:
        value = read()
        if value == 0:
            consume(value)
        elif value == 1:
            consume(-value)
        elif value == 2:
            break
    consume(None)
",
        "terminal_elif.py",
    );
}

#[test]
fn terminal_nested_while() {
    common::assert_matches_cpython_marshal(
        r"def terminal_nested_while(read, pending, consume):
    while True:
        value = read()
        if value is None:
            break
        while pending():
            consume(value)


def terminal_nested_while_with_following(read, pending, consume):
    while True:
        value = read()
        if value is None:
            break
        while pending():
            consume(value)
    consume(None)
",
        "terminal_nested_while.py",
    );
}

#[test]
fn protected_terminal_while() {
    common::assert_matches_cpython_marshal(
        r"async def protected_terminal_while(read, pending, consume):
    while True:
        value = await read()
        if value is None:
            break
        while pending():
            await consume(value)


def existing_iterator_control(items, action, consume):
    for value in items:
        try:
            action(value)
        finally:
            if value:
                consume(value)
",
        "protected_terminal_while.py",
    );
}

#[test]
fn nonfinal_loop_arms_preserve_chained_comparison_exits() {
    for (filename, source) in [
        (
            "nonfinal_for_chained_call.py",
            r"def target(items, enabled, low, high, consume):
    for item in items:
        if enabled:
            if low <= item < high:
                consume(item)
        else:
            consume(None)
",
        ),
        (
            "nonfinal_for_chained_return.py",
            r"def target(items, enabled, low, high):
    for item in items:
        if enabled:
            if low <= item < high:
                return item + 1
        else:
            if low <= item < high:
                return item
    return None
",
        ),
        (
            "nonfinal_while_chained_call.py",
            r"def target(read, enabled, low, high, consume):
    while True:
        item = read()
        if enabled:
            if low <= item < high:
                consume(item)
        else:
            consume(None)
",
        ),
        (
            "nonfinal_protected_chained_call.py",
            r"def target(items, enabled, low, high, consume):
    try:
        for item in items:
            if enabled:
                if low <= item < high:
                    consume(item)
            else:
                consume(None)
    except Exception:
        consume(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn nonfinal_chained_comparisons_preserve_copied_small_returns() {
    for (filename, source) in [
        (
            "nonfinal_chain_small_literal_return.py",
            r"def target(items, enabled, low, high, consume):
    for item in items:
        if enabled:
            if low <= item < high:
                return True
        else:
            consume(item)
    return False
",
        ),
        (
            "nonfinal_chain_small_name_return.py",
            r"def target(items, enabled, low, high, consume):
    for item in items:
        if enabled:
            if low <= item < high:
                return item
        else:
            consume(item)
    return False
",
        ),
        (
            "nonfinal_chain_call_bounds_small_return.py",
            r"def target(rules, value, ignored, is_range, split):
    for rule in rules:
        if ignored(rule):
            continue
        if is_range(rule):
            low, high = split(rule)
            if int(low) <= value <= int(high):
                return True
        elif int(rule) == value:
            return True
    return False
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn conditional_finally_copied_continue_nested_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, action, consume):
    for value in items:
        try:
            if not action(value):
                continue
        finally:
            if value:
                try:
                    consume(value)
                except Exception:
                    pass
",
        "finally_copied_continue_nested_handler.py",
    );
}

#[test]
fn conditional_finally_nested_handler_backedge() {
    for (filename, source) in [
        (
            "finally_linear_cleanup_control.py",
            r"def f(items, action, consume):
    for value in items:
        try:
            action(value)
        finally:
            if value:
                consume(value)
",
        ),
        (
            "finally_nested_handler_backedge.py",
            r"def f(items, action, consume):
    for value in items:
        try:
            action(value)
        finally:
            if value:
                try:
                    consume(value)
                except Exception:
                    pass
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
