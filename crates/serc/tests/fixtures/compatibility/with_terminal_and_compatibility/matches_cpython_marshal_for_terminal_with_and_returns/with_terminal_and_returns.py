def call_pair(manager, enabled, data):
    if not enabled:
        prepare()
    else:
        with manager:
            value = read(data)
            if first(value) and second(value):
                return False
    return True

def name_call(manager, value):
    with manager:
        reply = load(value)
        if reply and reply.get("success"):
            return reply
    return None

def call_compare(manager, value):
    with manager:
        if ready(value) and read(value) == "done":
            return True
    return False

def compare_pair(manager, value, expected):
    with manager:
        if value is not None and value == expected:
            return True
    return False

def negated_calls(manager, force):
    with manager:
        if not enabled() and not force:
            return False
    return True
