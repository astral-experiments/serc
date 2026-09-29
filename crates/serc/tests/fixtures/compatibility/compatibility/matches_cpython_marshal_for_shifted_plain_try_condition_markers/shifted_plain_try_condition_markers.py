def retained(items):
    for item in items:
        consume(item, 0)
        consume(item, 1)
        consume(item, 2)
        consume(item, 3)
        consume(item, 4)
        consume(item, 5)
        consume(item, 6)
        consume(item, 7)
        try:
            value = item.make()
            if value.flags.owned:
                continue
            value = value.view(Type)
        except Error:
            continue
        use(value)


def shifted(items):
    for item in items:
        consume(item, 0)
        consume(item, 1)
        consume(item, 2)
        consume(item, 3)
        consume(item, 4)
        consume(item, 5)
        consume(item, 6)
        consume(item, 7)
        consume(item, 8)
        try:
            value = item.make()
            if value.flags.owned:
                continue
            value = value.view(Type)
        except Error:
            continue
        use(value)


def direct_condition(items, first, second):
    for item in items:
        consume(first, second)
        consume(first, second)
        try:
            if item:
                continue
            use(item)
        except Error:
            continue
