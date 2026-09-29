
def call_control_following(flag):
    try:
        result = produce()
    finally:
        if flag:
            consume()
    return result
