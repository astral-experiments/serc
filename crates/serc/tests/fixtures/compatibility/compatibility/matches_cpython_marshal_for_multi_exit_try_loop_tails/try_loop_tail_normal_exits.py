def conditional(items):
    for item in items:
        try:
            if first(item):
                consume(item)
            elif second(item):
                consume_other(item)
        except LookupError:
            recover(item)


def complete_branch(items):
    for item in items:
        try:
            if accepted(item):
                consume(item)
            else:
                reject(item)
        except LookupError:
            recover(item)


def single_exit_return(items):
    for item in items:
        try:
            if accepted(item):
                return item
        except LookupError:
            continue


def single_exit_break(items):
    for item in items:
        try:
            if accepted(item):
                break
        except LookupError:
            continue
