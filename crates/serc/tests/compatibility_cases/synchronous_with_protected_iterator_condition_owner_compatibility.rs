use crate::common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_protected_iterator_condition_owners() {
    let source = r"with module_manager:
    for module_original in module_values:
        module_value = module_make(module_original)
        if not module_value:
            continue
        module_touch(module_value)


def attribute_continue(manager, values, make, touch):
    with manager:
        for original in values:
            value = make(original)
            if not value.ready:
                continue
            touch(value)


def nested_attribute_continue(first, second, values, make, touch):
    with first:
        with second:
            for original in values:
                value = make(original)
                if not value.inner.ready:
                    continue
                touch(value)


def attribute_body(manager, values, make, touch):
    with manager:
        for original in values:
            value = make(original)
            if not value.ready:
                touch(value)
            touch(value)


def name_body_assign(manager, values, make, touch):
    with manager:
        for original in values:
            value = make(original)
            if value:
                value = make(value)
            touch(value)


def boolean_none_break(manager, values, make, touch, limit):
    with manager:
        for value in values:
            if value.ready is not None and make(value) >= limit:
                break
            touch(value)


def boolean_or_break(manager, values, make, touch):
    with manager:
        for value in values:
            if value.ready is None or make(value):
                break
            touch(value)


def no_following_control(manager, lines, parse):
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue


def prior_for_control(manager, groups, lines, parse, use):
    with manager:
        for line in lines:
            for group in groups:
                use(group)
            value = parse(line)
            if not value:
                continue
            use(value)


def in_loop_marker_control(manager, lines, parse, use, flag):
    with manager:
        for line in lines:
            if flag:
                use(line)
            value = parse(line)
            if not value:
                continue
            use(value)


def positive_name_control(manager, lines, parse, use):
    with manager:
        for line in lines:
            value = parse(line)
            if value:
                continue
            use(value)


def negated_call_control(manager, lines, ready, use):
    with manager:
        for line in lines:
            if not ready(line):
                continue
            use(line)


def inner_try_control(manager, lines, parse, use):
    with manager:
        for line in lines:
            try:
                value = parse(line)
                if not value:
                    continue
                use(value)
            except ValueError:
                use(line)


def generator_control(manager, lines, parse, use):
    yield None
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


async def coroutine_control(manager, lines, parse, use, pause):
    await pause()
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


async def coroutine_break_control(manager, lines, use, pause, enabled, limit):
    with manager:
        for index, line in enumerate(lines):
            await pause(line)
            if enabled and index >= limit:
                break
            use(line)


def generator_break_yield(manager, lines, use, enabled, limit):
    with manager:
        for index, line in enumerate(lines):
            yield line
            if enabled and index >= limit:
                break
            use(line)


async def async_generator_break_yield(manager, lines, use, enabled, limit):
    with manager:
        for index, line in enumerate(lines):
            yield line
            if enabled and index >= limit:
                break
            use(line)


async def async_generator_break_await_control(manager, lines, use, pause, enabled, limit):
    yield None
    with manager:
        for index, line in enumerate(lines):
            await pause(line)
            if enabled and index >= limit:
                break
            use(line)


async def async_with_control(manager, lines, parse, use):
    async with manager:
        for line in lines:
            value = await parse(line)
            if not value:
                continue
            use(value)


def ownerless_control(lines, parse, use):
    for line in lines:
        value = parse(line)
        if not value:
            continue
        use(value)
";

    common::assert_matches_cpython_marshal(
        source,
        "<synchronous-with-protected-iterator-condition-owners>",
    );
}

#[test]
fn atomic_loop_tail_exclusions_subsume_only_their_marker() {
    for (source, filename) in [
        (
            "def f(items, manager):\n    with manager:\n        try:\n            work()\n        finally:\n            for item in items:\n                if item.left == LIMIT and item.right == \"ready\":\n                    consume(item)\n",
            "manager_finally_compound_overlap.py",
        ),
        (
            "def f(items, manager):\n    with manager:\n        for item in items:\n            flag = check(item)\n            if flag:\n                consume(item)\n",
            "manager_tail_simple.py",
        ),
        (
            "def f(items, manager):\n    with manager:\n        try:\n            work()\n        finally:\n            for item in items:\n                if check(item):\n                    consume(item)\n",
            "manager_finally_tail_call.py",
        ),
        (
            "def f(items, manager):\n    with manager:\n        for item in items:\n            if check(item):\n                consume(item)\n            finish(item)\n",
            "manager_forward_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn normal_finally_none_marker_preserves_compacted_try_prefix() {
    for (source, filename) in [
        (
            "def f(manager, flag, first, second, third):\n    def capture():\n        return flag\n    with manager:\n        try:\n            left = int(first.value)\n            middle = int(second.value)\n            right = int(third.value)\n        finally:\n            if flag is not None:\n                flag.close()\n    return left, middle, right, capture\n",
            "normal_finally_prior_try_owner.py",
        ),
        (
            "def f(manager, flag, metric, ready):\n    def capture():\n        return flag\n    with manager:\n        try:\n            if ready:\n                begin()\n            else:\n                prepare()\n            a = int(metric.value)\n            b = int(metric.value)\n            c = int(metric.value)\n            d = int(metric.value)\n            e = int(metric.value)\n            f = int(metric.value)\n            g = int(metric.value)\n        finally:\n            if flag is not None:\n                flag.close()\n    return a, b, c, d, e, f, g, capture\n",
            "normal_finally_prior_try_join.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
