def retry(flag, retries):
    attempt = 0
    while True:
        try:
            if flag:
                return first()
            return second()
        except Error as error:
            if attempt >= retries:
                raise
            attempt += 1
            report(error)


def no_loop_control(flag):
    try:
        if flag:
            return first()
        return second()
    except Error as error:
        report(error)


def iterator_loop_control(items, flag):
    for item in items:
        try:
            if flag:
                return first(item)
            return second(item)
        except Error as error:
            report(error)


def conditional_while_control(keep, flag):
    while keep():
        try:
            if flag:
                return first()
            return second()
        except Error as error:
            report(error)
