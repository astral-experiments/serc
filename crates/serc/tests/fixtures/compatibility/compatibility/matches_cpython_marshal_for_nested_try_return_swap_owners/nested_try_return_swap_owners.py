def target(var):
    try:
        return var['kindselector']['*']
    except KeyError:
        try:
            return var['kindselector']['kind']
        except KeyError:
            pass

def direct(var):
    try:
        return var['kindselector']['*']
    except KeyError:
        return var['kindselector']['kind']

def named(var):
    try:
        return var['kindselector']['*']
    except KeyError as error:
        try:
            return var['kindselector']['kind']
        except KeyError:
            pass

def literal(var):
    try:
        action()
    except KeyError:
        try:
            return False
        except TypeError:
            pass
