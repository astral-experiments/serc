def guarded(guard, choice, value, other, work, sink):
    try:
        work()
    finally:
        if guard:
            sink(first=value, last=value if choice else other)
