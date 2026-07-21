def generator_or_attribute(items, fallback):
    for item in items:
        if item.value or fallback:
            yield item


def generator_or_name(items, fallback):
    for item in items:
        if item or fallback:
            yield item


def generator_or_method(items, fallback):
    for item in items:
        if item.ready() or fallback:
            yield item


def generator_or_later_global(items, fallback):
    for item in items:
        if item.value or bool(fallback):
            yield item


def generator_three_operands(items, first, second):
    for item in items:
        if item.value or first or second:
            yield item


def generator_effect_then_yield(items, fallback, touch):
    for item in items:
        if item.value or fallback:
            touch(item)
            yield item


def generator_yield_from(items, fallback):
    for item in items:
        if item.value or fallback:
            yield from item.values


async def coroutine_iterator(items, fallback, pause):
    for item in items:
        if item.value or fallback:
            await pause()


async def async_generator_iterator(items, fallback, pause):
    for item in items:
        if item.value or fallback:
            await pause()
            yield item


def generator_and_control(items, fallback):
    for item in items:
        if item.value and fallback:
            yield item


def generator_first_global_control(items, fallback):
    for item in items:
        if bool(item) or fallback:
            yield item


def generator_global_attribute_control(items, fallback):
    for item in items:
        if type(item).__name__ or fallback:
            yield item


def generator_nested_global_control(items, fallback):
    for item in items:
        if bool(item.value).ready() or fallback:
            yield item


def generator_chained_comparison_control(items, fallback):
    for item in items:
        if 0 < item.value < 10 or fallback:
            yield item


def generator_elif_control(items, fallback, other):
    for item in items:
        if item.value or fallback:
            yield item
        elif other:
            yield other


def generator_else_control(items, fallback, other):
    for item in items:
        if item.value or fallback:
            yield item
        else:
            yield other


def generator_effect_control(items, fallback, touch):
    for item in items:
        if item.value or fallback:
            touch(item)
        yield item


def generator_return_control(items, fallback):
    for item in items:
        if item.value or fallback:
            return
        yield item


def generator_raise_control(items, fallback):
    for item in items:
        if item.value or fallback:
            raise RuntimeError
        yield item


def generator_continue_control(items, fallback):
    for item in items:
        if item.value or fallback:
            continue
        yield item


def generator_break_control(items, fallback):
    for item in items:
        if item.value or fallback:
            break
        yield item


def generator_manager_control(items, fallback, manager):
    for item in items:
        with manager():
            if item.value or fallback:
                yield item


async def coroutine_suspending_condition_control(items, fallback, pause):
    for item in items:
        if await pause(item.value) or fallback:
            yield item


def synchronous_control(items, fallback, touch):
    for item in items:
        if item.value or fallback:
            touch(item)
