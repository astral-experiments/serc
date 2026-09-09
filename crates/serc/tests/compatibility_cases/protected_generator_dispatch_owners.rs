use crate::common;

#[test]
fn preserves_dispatch_owner_slots_across_deferred_constants() {
    for definition in ["def", "async def"] {
        for callable in ["any", "all", "tuple"] {
            let source = format!(
                "{definition} process(manager, values, ready):\n    with manager:\n        assert ready\n        result = values[-1]\n        result = {callable}(value for value in values)\n        consume(result)\n",
            );
            common::assert_matches_cpython_marshal(&source, "deferred_dispatch_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_marshal_for_protected_generator_dispatch_owners() {
    for callable in ["any", "all", "tuple"] {
        let source = format!(
            r"def after_assert(manager, values, ready):
    with manager:
        assert ready
        result = {callable}(value for value in values)
        consume(result)


def nested_try(manager, values, ready):
    try:
        with manager:
            if ready:
                raise Error
            result = {callable}(value for value in values)
            consume(result)
    finally:
        cleanup()


def iterator_branch(manager, groups, ready):
    with manager:
        for values in groups:
            if ready:
                raise Error
            result = {callable}(value for value in values)
            consume(result)


async def asynchronous_manager(manager, values, ready):
    async with manager:
        assert ready
        result = {callable}(value for value in values)
        consume(result)


def initial_slot_control(manager, values):
    with manager:
        result = {callable}(value for value in values)
        consume(result)


def call_slot_control(manager, values, ready):
    with manager:
        assert ready
        prepare()
        result = {callable}(value for value in values)
        consume(result)
"
        );
        common::assert_matches_cpython_marshal(&source, "protected_generator_dispatch_owners.py");
    }
}

#[test]
fn matches_cpython_marshal_for_coroutine_comprehension_generator_dispatch_owners() {
    let source = r"async def boolean_filter(groups, ready):
    return [values for values in groups if ready or any(value for value in values)]


async def direct_filter(groups):
    return [values for values in groups if all(value for value in values)]


async def argument_control(groups):
    return [values for values in groups if consume(any(value for value in values))]
";
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_comprehension_generator_dispatch_owners.py",
    );
}

#[test]
fn matches_cpython_marshal_for_iterator_tuple_dispatch_after_branch() {
    let source = r"def after_branch(groups, ready):
    try:
        for values in groups:
            if ready:
                raise Error
            result = tuple(value for value in values)
            consume(result)
    except Error:
        pass


def first_iterator_slot_control(groups):
    try:
        for values in groups:
            result = tuple(value for value in values)
            consume(result)
    except Error:
        pass
";
    common::assert_matches_cpython_marshal(source, "iterator_tuple_dispatch_after_branch.py");
}
