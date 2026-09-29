def sync_split(items, expected, first, second):
    for item in items:
        assert (
            first(item)
            if item == expected
            else second(item)
        )


def sync_same_line(items, condition, first, second):
    for item in items:
        assert first(item) if condition else second(item)


async def coroutine_split(wait, items, expected, first, second):
    await wait()
    for item in items:
        assert (
            first(item)
            if item == expected
            else second(item)
        )


def generator_split(items, expected, first, second):
    yield first
    for item in items:
        assert (
            first(item)
            if item == expected
            else second(item)
        )


def call_selector_non_tail_control(items, condition, first, second):
    for item in items:
        assert first(item) if condition(item) else second(item)
        consume(item)


def non_final_position_control(items, selected, expected, first, second):
    for item in items:
        if selected(item):
            assert first(item) if item == expected else second(item)
            consume(item)
        else:
            reject(item)
