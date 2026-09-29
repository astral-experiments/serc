def process(kind, outer, middle, inner, value):
    match kind:
        case 'first':
            if outer:
                if middle:
                    if inner:
                        consume(value)
                else:
                    alternate(value)
        case _:
            fallback(value)
    finish(value)
