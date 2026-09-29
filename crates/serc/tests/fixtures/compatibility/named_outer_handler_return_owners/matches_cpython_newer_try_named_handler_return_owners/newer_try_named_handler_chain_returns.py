def run(first, second, third, fourth, flag):
    try:
        try:
            first()
        except LookupError as first_error:
            try:
                if flag:
                    return None
                return second()
            except ValueError as second_error:
                try:
                    return third()
                except KeyError as third_error:
                    try:
                        return fourth()
                    except RuntimeError as fourth_error:
                        return None
    except Exception:
        return False
