def short_circuit_break():
    while True:
        try:
            if first() or second():
                break
        except BlockingIOError:
            pass


def single_condition_break():
    while True:
        try:
            if first():
                break
        except BlockingIOError:
            pass


def short_circuit_continue():
    while True:
        try:
            if first() or second():
                continue
        except BlockingIOError:
            pass
