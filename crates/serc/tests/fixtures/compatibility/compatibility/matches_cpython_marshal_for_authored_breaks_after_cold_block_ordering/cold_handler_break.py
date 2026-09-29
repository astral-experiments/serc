def authored_break(items):
    for item in items:
        try:
            action(item)
        except Error:
            recover(item)
            break
        use(item)
    after()


def synthetic_cold_fallthrough(items):
    for item in items:
        try:
            action(item)
        except Error:
            recover(item)
        use(item)
    after()
