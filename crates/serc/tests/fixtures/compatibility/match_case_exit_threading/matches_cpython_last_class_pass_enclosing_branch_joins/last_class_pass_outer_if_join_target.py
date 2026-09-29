def sample(enabled, value, sink):
    if enabled:
        match value:
            case Empty():
                pass
            case _:
                sink.append(value)
    else:
        sink.append(None)
    result = sink.copy()
    sink.append(len(result))
    return result
