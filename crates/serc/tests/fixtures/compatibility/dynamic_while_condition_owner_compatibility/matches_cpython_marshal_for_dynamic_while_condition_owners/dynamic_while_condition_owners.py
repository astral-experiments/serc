
def chunk_style(iterable, size, take):
    iterator = iter(iterable)
    while batch := tuple(take(iterator, size)):
        yield batch


def group_style(stream, load):
    while count := read_int(stream):
        if count == 1:
            yield load(stream)
        elif count > 0:
            raise ValueError(count)


def cogroup_style(stream, load):
    while count := read_int(stream):
        if count == 2:
            yield (list(load(stream)), list(load(stream)))
        elif count > 0:
            raise ValueError(count)


def nested_keyword_style(context, version, index):
    while key := find_key(context, name=f"key-{version + index}", required=False):
        yield key


def pair_condition(next_value, first, second):
    while value := next_value(first, second):
        yield value


async def coroutine_global_condition(first):
    while value := GLOBAL(first):
        await value


async def async_generator_pair_condition(next_value, first, second):
    while value := next_value(first, second):
        yield value


def protected_try_condition(next_value, first, second):
    try:
        while value := next_value(first, second):
            yield value
    except LookupError:
        yield None


def nested_try_condition(next_value, first, second):
    try:
        try:
            while value := next_value(first, second):
                yield value
        except LookupError:
            yield None
    except Exception:
        yield None


def protected_with_condition(next_value, first, second, manager):
    with manager:
        while value := next_value(first, second):
            yield value


def nested_with_condition(next_value, first, second, outer, inner):
    with outer:
        with inner:
            while value := next_value(first, second):
                yield value


async def protected_async_with_condition(next_value, first, second, manager):
    async with manager:
        while value := next_value(first, second):
            await value


def ordinary_try_condition(next_value, first, second, observe):
    try:
        while value := next_value(first, second):
            observe(value)
    except LookupError:
        return None


def ordinary_with_condition(next_value, first, second, observe, manager):
    with manager:
        while value := next_value(first, second):
            observe(value)


def split_line_pair_control(next_value, first, second):
    while value := next_value(
        first,
        second,
    ):
        yield value


def high_local_pair_control(next_value, a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q):
    while value := next_value(p, q):
        yield value


def checked_pair_control(next_value, first, second, ready):
    if ready:
        left = first
        right = second
    while value := next_value(left, right):
        yield value


def body_pair_control(next_value, first, second, observe):
    while value := next_value(first):
        yield observe(first, second, value)


def direct_comparison_control(first, second):
    while first < second:
        yield first


async def awaited_condition_control(next_value, first, second):
    while value := await next_value(first, second):
        await value


def inline_argument_control(next_value, first, second):
    while value := next_value(first and second):
        yield value


def comprehension_argument_control(next_value, items):
    while value := next_value([item for item in items if item]):
        yield value


def tail_nested_control(next_value, first, second, items):
    for item in items:
        while value := next_value(first, second, item):
            yield value


async def name_condition_regression_control(conns):
    while conns:
        await conns.pop()


def with_attribute_condition_regression_control(manager, state):
    with manager:
        while state.waiting:
            state.consume()


def with_method_condition_regression_control(manager, state):
    with manager:
        while state.has_process():
            state.kill()


async def finally_attribute_condition_regression_control(callbacks):
    try:
        await callbacks.prepare()
    finally:
        while callbacks.done:
            await callbacks.pop()()


def finally_method_condition_regression_control(queue):
    try:
        queue.prepare()
    finally:
        while not queue.empty():
            queue.get()


def nested_suspension_condition_regression_control(pgconn):
    while pgconn.flush() != 0:
        while not (ready := (yield WAIT_RW)):
            continue
        yield ready
