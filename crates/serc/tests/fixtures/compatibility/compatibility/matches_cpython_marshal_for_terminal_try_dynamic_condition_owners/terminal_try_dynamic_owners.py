def call_then_local():
    try:
        flag = source()
        if flag:
            action()
    except Exception:
        recover()

def unpack_then_local(values):
    try:
        first, second = values
        if first:
            action()
    except Exception:
        recover()

def fast_pair(left, right):
    try:
        if left + right:
            action()
    except Exception:
        recover()

def fast_const(left):
    try:
        if left + 1:
            action()
    except Exception:
        recover()

def global_call():
    try:
        if predicate():
            action()
    except Exception:
        recover()

def local_call(predicate):
    try:
        if predicate():
            action()
    except Exception:
        recover()

def method_call(obj):
    try:
        if obj.predicate():
            action()
    except Exception:
        recover()

def fast_dict(key, value):
    try:
        if {key: value}:
            action()
    except Exception:
        recover()

def constant_dict(value):
    try:
        if {'key': value}:
            action()
    except Exception:
        recover()

def unpack_dict(mapping):
    try:
        if {**mapping}:
            action()
    except Exception:
        recover()

def prior_call_then_while(flag):
    try:
        seed()
        while flag:
            action()
            break
    except Exception:
        recover()
