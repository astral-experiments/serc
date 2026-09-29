def outer(subject):
    def f():
        try:
            match subject:
                case "left":
                    return 1
                case _:
                    return 0
        except Exception:
            return None
    return f
