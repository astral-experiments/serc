def case_0(items, first, second, third, manager):
    with manager:
        for item in items:
            if first:
                if second:
                    value = 1
                    if previous:
                        previous_action()
                    elif first and third:
                        process()
                else:
                    alternate()
            else:
                final()


def case_1(items, first, second, third, manager):
    try:
        for item in items:
            if first:
                if second:
                    value = 1
                    if previous:
                        previous_action()
                    elif first or third:
                        process()
                else:
                    alternate()
            else:
                final()
    except Error:
        cleanup()
