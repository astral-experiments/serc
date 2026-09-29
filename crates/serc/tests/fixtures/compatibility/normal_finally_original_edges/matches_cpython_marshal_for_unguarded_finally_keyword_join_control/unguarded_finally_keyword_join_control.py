def unguarded(guard, choice, value, other, work, sink):
    try:
        work()
    finally:
        sink(first=value, last=value if choice else other)
