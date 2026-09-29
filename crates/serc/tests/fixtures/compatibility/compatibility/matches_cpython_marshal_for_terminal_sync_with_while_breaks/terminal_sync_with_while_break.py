def nested_tail_break(manager, enabled, stream):
    with manager:
        if enabled:
            line = '\n'
            position = stream.tell()
            while line:
                line = stream.readline()
                if '[' in line:
                    stream.seek(position)
                    break
                position = stream.tell()
        finish(stream)

def nested_break_only(manager, active, items, stop):
    with manager:
        while active:
            for item in items:
                if stop(item):
                    break
            step()
