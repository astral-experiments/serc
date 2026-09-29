
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            return
    finish(value)
