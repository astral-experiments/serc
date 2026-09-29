
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad:
            raise
    finish(value)
