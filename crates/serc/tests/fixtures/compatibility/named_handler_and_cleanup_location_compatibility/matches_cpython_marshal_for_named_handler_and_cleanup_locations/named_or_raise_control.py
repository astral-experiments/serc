
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad or error.ready:
            raise
    finish(value)
