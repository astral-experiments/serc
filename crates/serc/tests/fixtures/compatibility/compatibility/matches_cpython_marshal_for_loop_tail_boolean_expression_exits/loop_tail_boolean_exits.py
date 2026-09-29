def conjunction(items):
    for item in items:
        left(item) and right(item)


def disjunction(items):
    for item in items:
        first(item) or second(item) or third(item)


def non_tail(items):
    for item in items:
        left(item) and right(item)
        consume(item)


def final_call(items):
    for item in items:
        consume(item)


def assigned(items):
    for item in items:
        result = left(item) and right(item)
