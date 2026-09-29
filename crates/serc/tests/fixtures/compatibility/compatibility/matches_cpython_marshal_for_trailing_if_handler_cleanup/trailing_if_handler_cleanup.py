def unknown_name_test(flag):
    try:
        action()
    except Error:
        if flag:
            recover()
    after()


def compare_test(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        if key not in mapping.values():
            print(key)
    value = key


def named_call_body(mapping, key):
    try:
        value = mapping[key]
    except KeyError as error:
        if key not in mapping.values():
            print(error)
    value = key


def bare_call_body(mapping, key):
    try:
        value = mapping[key]
    except:
        if key not in mapping.values():
            print(key)
    value = key


def explicit_else_control(flag):
    try:
        action()
    except Error as error:
        if flag:
            consume(error)
        else:
            finish()


def direct_tail(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        print(key)
    value = key


def noncall_control(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        if key not in mapping.values():
            raise
    value = key


def multiple_statement_control(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        prepare()
        if key not in mapping.values():
            print(key)
        finish()
    value = key


def nontrailing_control(mapping, key):
    try:
        value = mapping[key]
    except KeyError:
        if key not in mapping.values():
            print(key)
        finish()
    value = key
