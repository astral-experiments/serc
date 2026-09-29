
def name_return_control(flag):
    result = produce()
    try:
        return result
    finally:
        if flag:
            consume()
