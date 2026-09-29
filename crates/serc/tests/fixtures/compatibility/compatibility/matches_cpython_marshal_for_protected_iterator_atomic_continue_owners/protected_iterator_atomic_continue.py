def call_condition(manager, lines):
    with manager:
        for line in lines:
            if line.strip().startswith("#"):
                continue
            consume(line)


def name_condition(manager, lines, initial):
    ignoring = initial
    with manager:
        for line in lines:
            if ignoring:
                if line:
                    ignoring = False
                continue
            if line.strip().startswith("#"):
                continue
            consume(line)

