def nested_positive(manager, outer, inner, predicates):
    with manager:
        for left in outer:
            left = normalize(left)
            for right in inner:
                right = normalize(right)
                if predicates.ready(left) and predicates.ready(right):
                    continue
                consume(left, right)


def first_positive(manager, lines):
    with manager:
        for line in lines:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            consume(line)


def multiline_condition(manager, values):
    with manager:
        for value in values:
            if (
                ignored(value)
                or hidden(value)
            ):
                continue
            consume(value)


def non_continue(manager, values):
    with manager:
        for value in values:
            if first(value) and second(value):
                consume(value)
            finish(value)


async def asynchronous(manager, values):
    async with manager:
        for value in values:
            if first(value) and second(value):
                continue
            consume(value)
