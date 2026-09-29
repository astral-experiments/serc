def inside_finally(items, first, second, third, manager):
    try:
        for item in items:
            if first:
                if second:
                    value = 1
                    if third:
                        process()
                else:
                    alternate()
            else:
                final()
    finally:
        cleanup()


def conjunction_in_except(items, first, second, third, manager):
    try:
        for item in items:
            if first:
                if second:
                    value = 1
                    if first and third:
                        process()
                else:
                    alternate()
            else:
                final()
    except Error:
        cleanup()


def disjunction_in_with(items, first, second, third, manager):
    with manager:
        for item in items:
            if first:
                if second:
                    value = 1
                    if first or third:
                        process()
                else:
                    alternate()
            else:
                final()


def generator(items, first, second, third, manager):
    for item in items:
        if first:
            if second:
                value = 1
                if third():
                    process()
            else:
                alternate()
        else:
            final()
    yield None
