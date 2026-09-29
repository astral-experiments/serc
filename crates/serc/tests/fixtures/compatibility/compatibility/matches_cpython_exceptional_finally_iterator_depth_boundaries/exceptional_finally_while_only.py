def run(source, enabled):
    for outer in source:
        try:
            return outer
        finally:
            while enabled:
                return outer
