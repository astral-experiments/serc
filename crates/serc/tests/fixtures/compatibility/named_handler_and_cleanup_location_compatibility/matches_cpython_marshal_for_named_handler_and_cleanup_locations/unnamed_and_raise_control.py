
def check(value):
    try:
        use(value)
    except Error:
        if value.bad and value.ready:
            raise
    finish(value)
