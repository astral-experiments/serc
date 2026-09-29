def final_else(outer, inner):
    if outer:
        if inner:
            first()
        else:
            second()
    else:
        other()
    after()

def terminal_final_elif(outer, inner, last):
    if outer:
        if inner:
            first()
        elif last:
            raise Error()
    else:
        other()
    after()

def nonterminal_final_elif(outer, inner, last):
    if outer:
        if inner:
            raise Error()
        elif last:
            body()
    else:
        other()
    after()

def terminal_final_else(outer, inner):
    if outer:
        if inner:
            first()
        else:
            raise Error()
    else:
        other()
    after()

def no_else_nonterminal(outer, inner):
    if outer:
        if inner:
            body()
    else:
        other()
    after()

def folded_nested(outer):
    if outer:
        if True:
            body()
    else:
        other()
    after()

def threaded_three_way_cascade(major, minor, bugfix, other_major, other_minor, other_bugfix):
    if major == other_major:
        if minor == other_minor:
            if bugfix == other_bugfix:
                result = 0
            elif bugfix > other_bugfix:
                result = 1
            else:
                result = -1
        elif minor > other_minor:
            result = 1
        else:
            result = -1
    elif major > other_major:
        result = 1
    else:
        result = -1
    return result

def cascade_with_leading_statement(outer, middle, inner):
    if outer:
        prepare()
        if middle:
            if inner:
                first()
            elif alternate():
                second()
            else:
                third()
        elif alternate():
            fourth()
        else:
            fifth()
    elif alternate():
        sixth()
    else:
        seventh()
    after()
