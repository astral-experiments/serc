
def call_positive(flag):
    try:
        return produce()
    finally:
        if flag:
            consume()
