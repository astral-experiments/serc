def for_loop(items):
    for item in items:
        if classify(item) == "first":
            if item:
                consume(item)
        elif classify(item) == "second":
            if item is not None:
                consume(item)
        else:
            fallback(item)


def while_loop(items):
    while items:
        item = items.pop()
        if is_first(item):
            if item:
                consume(item)
        elif is_second(item):
            consume(item)


def recursively_nested(items):
    for item in items:
        if is_first(item):
            if item:
                if accepted(item):
                    consume(item)
        elif is_second(item):
            consume(item)


def final_elif_in_non_final_branch(items):
    for item in items:
        if is_first(item):
            if accepted(item):
                consume(item)
            elif deferred(item):
                if ready(item):
                    consume(item)
        else:
            fallback(item)
