seed = 0
if True:
    body()
def between():
    pass
reused = True

if a:
    if b:
        body()
elif c:
    other()
after1(); after2(); after3(); after4(); after5(); after6()

if x:
    if True:
        body()
elif y:
    other()

if False if True else False:
    body()
elif True:
    other()
else:
    unreachable()

def conditional_in_loop(src, dst):
    for k, v in src:
        if True if True else False:
            dst[k] = v

def terminating_nested_if(x, y):
    if x:
        if y:
            raise Exception()
        else:
            body()
    else:
        other()
    after()

def nested_if_before_implicit_return(x, y):
    if x:
        if y:
            return
        else:
            value = 2
    else:
        value = 3
    return

def nested_if_with_fallthrough_return(x, y):
    if x:
        if y:
            return y
    else:
        return x
    return None

def nested_if_before_loop_control(items, condition):
    for item in items:
        if item:
            if condition:
                value = 1
            else:
                value = 2
            continue
        else:
            value = 3
        continue

def protected_loop_control(items):
    for item in items:
        try:
            body()
            if item:
                break
        except Exception:
            recover()
    for item in items:
        try:
            body()
            continue
        except Exception:
            recover()

def nested_break_then_return(items, condition):
    for item in items:
        if item:
            if condition:
                break
        else:
            return condition
        return None

if True:
    if first:
        one()
    elif second:
        two()
