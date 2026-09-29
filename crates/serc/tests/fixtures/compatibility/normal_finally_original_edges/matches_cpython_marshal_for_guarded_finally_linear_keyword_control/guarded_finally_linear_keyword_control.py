def guarded_linear(guard, choice, value, other, work, sink):
    try:
        work()
    finally:
        if guard:
            sink(first=value, last=other)
