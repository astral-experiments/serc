def simplify(items, selected, ready, compatible, state):
    for item in items:
        if selected(item):
            value = item.value
            if ready(value):
                for child in item.children:
                    if ready(child) or compatible(value, child):
                        if compatible(child, value):
                            state[child] = value
                        else:
                            break
                    else:
                        break
                else:
                    continue
        elif item.kind == 1:
            state[item] = None
        consume(item)
    return state
