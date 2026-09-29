def choose(outer, inner, ready):
    match outer:
        case Outer.FIRST:
            values = []
        case Outer.NESTED:
            values = []
            match inner:
                case Inner.FIRST:
                    if ready:
                        values.append(1)
                case Inner.SECOND:
                    if ready:
                        values.append(2)
                case Inner.LAST:
                    pass
        case _:
            values = []
    consume(values)
    return values
