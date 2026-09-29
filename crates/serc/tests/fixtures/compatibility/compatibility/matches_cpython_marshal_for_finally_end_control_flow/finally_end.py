def conditional_return(flag):
    try:
        pass
    finally:
        if flag:
            return

def nested_return():
    try:
        try:
            pass
        finally:
            return
    finally:
        pass

def nested_handler_return():
    try:
        pass
    finally:
        try:
            return
        except Exception:
            pass

def break_from_finally():
    while True:
        try:
            pass
        finally:
            break

def continue_from_finally():
    while True:
        try:
            pass
        finally:
            continue
