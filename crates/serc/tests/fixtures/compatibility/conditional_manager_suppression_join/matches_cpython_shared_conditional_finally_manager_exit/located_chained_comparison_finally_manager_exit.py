def cleanup(outer, inner, work, remove, low, value, high):
    with outer():
        try:
            work(value)
        finally:
            if low < value < high:
                with inner():
                    remove(value)
    return work(value)
