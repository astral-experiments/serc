def body_if(flag):
    try:
        if flag:
            action()
    except Exception:
        pass

def leading_and_terminal_if(first, second):
    try:
        if first:
            before()
        if second:
            action()
    except Exception:
        pass

def body_if_else(flag):
    try:
        if flag:
            action()
        else:
            other()
    except Exception:
        pass

def body_while(flag):
    try:
        while flag:
            action()
            break
    except Exception:
        pass

def body_match(value):
    try:
        match value:
            case 1:
                action()
    except Exception:
        pass

def else_if(flag):
    try:
        action()
    except Exception:
        recover()
    else:
        if flag:
            success()

def conditional_expression(flag):
    try:
        action() if flag else other()
    except Exception:
        pass

def boolean_expression(flag):
    try:
        flag and action()
    except Exception:
        pass

def chained_comparison(left, middle, right):
    try:
        left < middle < right
    except Exception:
        pass

def tuple_conditional_expression(flag):
    try:
        (before(), action() if flag else other())
    except Exception:
        pass

async def coroutine_if(flag):
    try:
        if flag:
            await action()
    except Exception:
        pass
