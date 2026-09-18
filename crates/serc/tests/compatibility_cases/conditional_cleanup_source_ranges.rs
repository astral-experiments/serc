use crate::common;

#[test]
fn preserves_folded_named_handler_cleanup_locations() {
    for (filename, source) in [
        (
            "named_handler_true_boolean_assignment.py",
            r"def sample(request, ready):
    response = None
    try:
        response = request()
    except Error as error:
        if True:
            response = None
    return response
",
        ),
        (
            "named_handler_true_string_assignment.py",
            r#"def sample(request, ready):
    response = None
    try:
        response = request()
    except Error as error:
        if "available":
            response = None
    return response
"#,
        ),
        (
            "named_handler_true_string_call.py",
            r#"def sample(request, consume):
    try:
        request()
    except Error as error:
        if "available":
            consume(error)
    return None
"#,
        ),
        (
            "named_handler_false_string_assignment.py",
            r#"def sample(request, ready):
    response = None
    try:
        response = request()
    except Error as error:
        if "":
            response = None
    return response
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_terminal_manager_return_ranges() {
    for (filename, source) in [
        (
            "chained_if_with_implicit_return.py",
            r#"def inspect(index, values, manager, emit):
    if 0 <= int(index) < len(values):
        with manager("selected", expanded=True):
            emit(values[int(index)])
"#,
        ),
        (
            "chained_if_with_multiple_managers.py",
            r"def inspect(low, value, high, first, second, emit):
    if low <= value < high:
        with first() as left, second() as right:
            emit(left, right, value)
",
        ),
        (
            "chained_if_with_multiline_manager.py",
            r#"def inspect(low, value, high, manager, emit):
    if low <= value < high:
        with manager(
            "selected",
            expanded=True,
        ) as selected:
            emit(selected, value)
"#,
        ),
        (
            "chained_if_async_with_implicit_return.py",
            r"async def inspect(low, value, high, manager, emit):
    if low <= value < high:
        async with manager():
            await emit(value)
",
        ),
        (
            "chained_if_with_explicit_return_control.py",
            r"def inspect(low, value, high, manager):
    if low <= value < high:
        with manager():
            return value
",
        ),
        (
            "chained_if_with_following_statement_control.py",
            r"def inspect(low, value, high, manager, emit):
    if low <= value < high:
        with manager():
            emit(value)
    emit(high)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_recursive_while_exit_locations() {
    for (filename, source) in [
        (
            "complete_else_shared_while_forward.py",
            r"def populate(selected, prepared, ready, pool, output, limit):
    if selected:
        if prepared:
            output.append(0)
        else:
            output.append(1)
            while len(output) < limit and pool:
                output.append(pool[len(output) % len(pool)])
    else:
        output.append(2)
    consume(output)
",
        ),
        (
            "complete_else_shared_while_authored_suffix_control.py",
            r"def populate(selected, prepared, ready, pool, output, limit):
    if selected:
        if prepared:
            output.append(0)
        else:
            output.append(1)
            while len(output) < limit and pool:
                output.append(pool[len(output) % len(pool)])
            checkpoint(output)
    else:
        output.append(2)
    consume(output)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
