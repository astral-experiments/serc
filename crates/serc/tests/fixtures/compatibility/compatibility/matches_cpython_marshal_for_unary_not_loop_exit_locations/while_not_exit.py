def terminal_call():
    while not predicate():
        work()

def terminal_nested_not():
    while not not predicate():
        work()

def terminal_and(flag):
    while flag and not predicate():
        work()

def protected(lock):
    with lock:
        while not lock.done():
            work()
    work()
