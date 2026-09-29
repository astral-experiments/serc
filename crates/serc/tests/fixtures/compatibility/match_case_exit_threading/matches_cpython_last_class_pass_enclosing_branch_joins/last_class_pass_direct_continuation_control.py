def sample(value, sink):
    match value:
        case Empty():
            pass
        case _:
            sink.append(value)
    result = sink.copy()
    sink.append(len(result))
    return result
