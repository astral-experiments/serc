def outer():
    "retained value"
    def inner():
        "retained value"
        return "retained value"
    return inner
