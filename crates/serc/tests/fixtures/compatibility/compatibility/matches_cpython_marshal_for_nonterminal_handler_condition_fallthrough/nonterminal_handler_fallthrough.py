def named_raise(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise error
    after()


def named_return(flag):
    try:
        action()
    except Error as error:
        if flag:
            return error
    after()


def bare_raise(flag):
    try:
        action()
    except Error:
        if flag:
            raise
    after()


def final_elif(first, second):
    try:
        action()
    except Error as error:
        if first:
            raise error
        elif second:
            raise error
    after()


def explicit_else(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise error
        else:
            consume(error)
    after()
