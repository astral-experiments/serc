
def f(manager, values, use):
    with manager:
        try:
            for value in values:
                if value is None:
                    continue
                try:
                    use(value)
                except LookupError:
                    break
        finally:
            use(None)
