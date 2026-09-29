def convert(items, selected, ready, output, state):
    skipped = 0
    last = None
    for item in items:
        if selected(item):
            try:
                if item.kind == 0:
                    output.append(item)
                    if ready(item):
                        state.append(item)
                        last = 0
                elif item.kind == 1:
                    if not state:
                        return None
                    output.append(item)
                    if ready(item):
                        state.append(item)
                        last = 1
                else:
                    skipped += 1
            except Error:
                skipped += 1
        else:
            output.append(item)
    return output, last, skipped
