def generator_bare_return(state, items):
    try:
        saved = [item for item in items if item.ready()]
        if not saved:
            return
        for item in items:
            yield item
    finally:
        state.close()


def ordinary_bare_return(state, items):
    try:
        saved = [item for item in items if item.ready()]
        if not saved:
            return
        consume(items)
    finally:
        state.close()


def generator_value_return(state, items, value):
    try:
        if value:
            return value
        for item in items:
            yield item
    finally:
        state.close()


def handler_none_return(state, value):
    try:
        if value:
            return None
        consume(value)
    except ValueError:
        consume(value)
    finally:
        state.close()
