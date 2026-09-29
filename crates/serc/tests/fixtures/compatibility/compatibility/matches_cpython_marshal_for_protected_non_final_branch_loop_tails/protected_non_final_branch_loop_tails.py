def protected_state_machine(items):
    for index, item in enumerate(items):
        try:
            if state == SCANNING:
                if item.startswith(tag):
                    if item.endswith(end):
                        state = STATE_RETTYPE
                    else:
                        state = STATE_DOC
            elif state == STATE_DOC:
                if item.startswith(end):
                    state = STATE_RETTYPE
                else:
                    output.append(item)
            elif state == STATE_RETTYPE:
                state = STATE_NAME
            elif state == STATE_NAME:
                state = STATE_ARGS
            elif state == STATE_ARGS:
                if item.startswith(start):
                    state = SCANNING
                else:
                    arguments.append(item)
        except ParseError:
            raise
        except Exception as error:
            message = 'details'
            raise ParseError(index, message) from error


def single_normal_exit_control(items):
    for item in items:
        try:
            if accepted(item):
                consume(item)
        except LookupError:
            continue
