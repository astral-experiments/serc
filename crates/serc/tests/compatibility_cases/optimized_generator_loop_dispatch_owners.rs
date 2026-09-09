use crate::common;

#[test]
fn matches_cpython_marshal_for_optimized_generator_loop_dispatch_owners() {
    let source = r"def while_guard(values, ready):
    try:
        while ready:
            found = any(value for value in values)
            use(found)
    except Error:
        pass


def constant_while_guard(values):
    try:
        while True:
            found = all(value for value in values)
            use(found)
    except Error:
        pass


def prior_call_keeps_slot(values, ready):
    try:
        while ready:
            prepare()
            found = any(value for value in values)
            use(found)
    except Error:
        pass


def prior_unpack_keeps_slot(values, ready, pair):
    try:
        while ready:
            left, right = pair
            found = all(value for value in values)
            use(left, right, found)
    except Error:
        pass


def iterator_entry_keeps_slot(values):
    try:
        for entry in values:
            found = any(value for value in entry)
            use(found)
    except Error:
        pass
";
    common::assert_matches_cpython_marshal(source, "optimized_generator_loop_dispatch_owners.py");
}

#[test]
fn matches_cpython_marshal_for_optimized_generator_coroutine_iterator_dispatch_owners() {
    let source = r"async def after_await(values, tick):
    for entry in values:
        await tick()
        found = any(value for value in entry)
        use(found)


async def after_assert(values, ready):
    for entry in values:
        assert ready
        found = all(value for value in entry)
        use(found)


async def iterator_entry_control(values, tick):
    await tick()
    for entry in values:
        if any(value for value in entry):
            break
        await tick()
";
    common::assert_matches_cpython_marshal(
        source,
        "optimized_generator_coroutine_iterator_dispatch_owners.py",
    );
}

#[test]
fn matches_cpython_marshal_for_optimized_generator_try_dispatch_slots() {
    for callable in ["any", "all"] {
        let source = format!(
            r"def prior_call(values):
    try:
        prepare()
        return {callable}(value for value in values)
    except Error:
        pass


def prior_unpack(values, pair):
    try:
        left, right = pair
        return {callable}(value for value in values)
    except Error:
        pass


def prior_comprehension(values):
    try:
        mapping = {{value: value for value in values}}
        return {callable}(value for value in values)
    except Error:
        pass


def first_guard_control(values):
    try:
        return {callable}(value for value in values)
    except Error:
        pass
"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "optimized_generator_try_dispatch_slots.py",
        );
    }
}

#[test]
fn matches_cpython_marshal_for_generator_guard_after_captured_comprehension_result() {
    let source = r"async def captured_result(values, keys, tick):
    try:
        values = await tick()
    finally:
        await tick()
    mapping = {str(row['key']): row for row in values}
    return tuple(mapping.get(key) for key in keys)
";
    common::assert_matches_cpython_marshal(source, "captured_comprehension_result.py");
}
