def recover():
    try:
        work()
    except Error as error:
        raise Failure from error
    else:
        pass
    finally:
        pass
    finish()
