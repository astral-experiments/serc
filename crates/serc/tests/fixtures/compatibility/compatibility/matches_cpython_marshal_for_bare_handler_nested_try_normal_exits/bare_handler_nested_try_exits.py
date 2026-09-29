def fallback_import():
    try:
        import primary
        state = 2
    except ImportError:
        try:
            import fallback
            state = 1
        except ImportError as error:
            report(error)
            state = 0
    consume(state)


def nested_try_with_following_handler_statement():
    try:
        value = first()
    except LookupError:
        try:
            value = second()
        except LookupError:
            value = fallback()
        consume(value)
    return value


def terminating_nested_handler(key):
    try:
        value = first[key]
    except KeyError:
        try:
            value = second[key]
        except KeyError:
            raise NameError(key) from None
    return value
