def run(manager, ready):
    with manager:
        try:
            while True:
                if step():
                    return 0 if ready else 1
        except StopIteration:
            return 2
