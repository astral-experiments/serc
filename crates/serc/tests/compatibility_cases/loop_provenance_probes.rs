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
