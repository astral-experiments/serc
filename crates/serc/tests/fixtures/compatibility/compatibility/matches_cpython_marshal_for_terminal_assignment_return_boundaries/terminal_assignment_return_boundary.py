def consecutive_handlers(flag):
    global state
    if flag:
        try:
            first()
        except FirstError:
            pass
        try:
            second()
        except SecondError:
            pass
        state = None


def explicit_else(flag):
    if flag:
        use(flag)
    else:
        try:
            value = first()
        except FirstError:
            pass
        value = None


def call_tail_control(flag):
    if flag:
        try:
            first()
        except FirstError:
            pass
        finish()


def nonterminal_control(flag):
    if flag:
        try:
            first()
        except FirstError:
            pass
        value = None
    finish()


def simple_assignment_control(flag):
    if flag:
        value = None
