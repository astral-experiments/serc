def sample(enabled, value, ready, sink):
    if enabled:
        match value:
            case str(text):
                sink.append(text)
            case Empty():
                pass
            case _:
                sink.append(value)
    elif ready:
        sink.clear()
    else:
        sink.append(None)
    result = sink.copy()
    sink.append(len(result))
    return result
