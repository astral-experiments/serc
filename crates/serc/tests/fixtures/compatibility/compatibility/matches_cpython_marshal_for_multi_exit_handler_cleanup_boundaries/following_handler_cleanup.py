def following_continuation(var):
    try:
        return 1 / var.some_other_func()
    except AttributeError:
        if (message := var.get_message()):
            pass
    except ZeroDivisionError:
        message = 'division by zero'
    print(message)
