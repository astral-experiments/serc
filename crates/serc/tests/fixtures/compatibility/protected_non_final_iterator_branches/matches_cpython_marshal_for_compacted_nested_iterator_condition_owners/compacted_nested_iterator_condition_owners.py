def case_0(items, first, second, third, manager):
    try:
        for row in items:
            for item in row:
                if first:
                    choice = factory()
                    if choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()
    finally:
        cleanup()


def case_1(items, first, second, third, manager):
    try:
        for row in items:
            for item in row:
                if first:
                    choice = factory()
                    if not choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()
    except Error:
        cleanup()


def case_2(items, first, second, third, manager):
    with manager:
        for row in items:
            for item in row:
                if first:
                    choice = second()
                    if choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()


def case_3(items, first, second, third, manager):
    try:
        for row in items:
            for item in row:
                if first:
                    choice = second()
                    if not choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()
    finally:
        cleanup()
    yield None
