def generator_calls(flag, items):
    if flag:
        assert items
    if flag:
        first = next(item for item in items if item)
        assert first
        last = next(item for item in items if not item)
        assert last
    else:
        assert not items


def completed_comprehensions(flag, items, other):
    if flag:
        assert items
    if flag:
        values = [item for item in (items if flag else other)]
        assert values
        unique = {item for item in values if item}
        assert unique
        mapping = {item: item for item in unique}
        assert mapping
    else:
        assert not items


async def nested_comprehensions(flag, groups):
    if flag:
        assert groups
    if flag:
        values = [[lambda: item for item in group] for group in groups]
        assert values
        first = next(item for group in groups for item in group)
        assert first
    else:
        assert not groups


def generator_default(flag, items):
    if flag:
        assert items
    if flag:
        first = next((item for item in items if item), None)
        assert first
    else:
        assert not items
