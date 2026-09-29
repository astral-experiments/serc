from __future__ import annotations


def target() -> None:
    value = make_value((2, 3), dtype)
    interim = make_value((1, 2, 3), dtype)
    result = make_value((1, 2, 2), dtype)
    first_calls: list[tuple[Item, object]] = []
    second_calls: list[tuple[Item, tuple[object, ...]]] = []

    def first(item: Item, shape: object) -> Item:
        first_calls.append((item, shape))
        return interim

    def second(item: Item, *, slices: tuple[object, ...]) -> Item:
        second_calls.append((item, slices))
        return result

    old_first = first_module.convert
    old_second = second_module.select
    first_module.convert = first
    second_module.select = second
    try:
        assert value[None, :, 1:] is result
    finally:
        first_module.convert = old_first
        second_module.select = old_second

    check_results(first_calls, [(value, Shape(1, 2, 3))])
    check_results(
        second_calls,
        [(interim, (slice(None, None, None), slice(None, None, None), slice(1, None, None)))],
    )
