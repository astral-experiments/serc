def run(source, enabled):
    for outer in source:
        try:
            return outer
        finally:
            if enabled:
                return convert(outer)
