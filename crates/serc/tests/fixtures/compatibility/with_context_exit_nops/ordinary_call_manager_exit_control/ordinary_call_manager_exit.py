def function(manager, value, fallback):
    with manager:
        fallback(value)
