def run(source, inner, enabled):
    for outer in source:
        try:
            return outer
        finally:
            while enabled:
                for item in inner:
                    return item
                break
