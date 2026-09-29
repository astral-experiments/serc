def nested_conditional_cleanup(path, left, right):
    try:
        try:
            return left == right
        finally:
            if path.exists():
                path.unlink()
    finally:
        release()


def three_nested_cleanups(value):
    try:
        try:
            try:
                return value
            finally:
                inner()
        finally:
            middle()
    finally:
        outer()


def single_cleanup_control(value):
    try:
        return value
    finally:
        cleanup()
