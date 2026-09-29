def run(source, inner, enabled):
    for outer in source:
        try:
            return outer
        finally:
            if enabled:
                for group in inner:
                    for item in group:
                        return item
