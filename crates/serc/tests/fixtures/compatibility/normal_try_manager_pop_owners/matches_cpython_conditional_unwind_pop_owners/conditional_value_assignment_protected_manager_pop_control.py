def run(manager, ready):
    with manager:
        try:
            while True:
                if step():
                    result = 0 if ready else 1
                    return result
        except StopIteration:
            return 2
