
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
        consume(error)
    finish(value)
