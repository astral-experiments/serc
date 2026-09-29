def unnamed(action, reraises):
    try:
        action()
    except Exception:
        if reraises:
            raise

def named(action, reraises):
    try:
        action()
    except Exception as error:
        if reraises(error):
            raise

def final_elif(action, first, second):
    try:
        action()
    except Exception:
        if first:
            raise
        elif second:
            raise
