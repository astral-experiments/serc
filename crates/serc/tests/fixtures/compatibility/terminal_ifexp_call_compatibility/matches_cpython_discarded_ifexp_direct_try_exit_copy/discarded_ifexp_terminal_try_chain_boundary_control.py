def target(command, choose, left, right):
    try:
        if command == "one":
            left() if choose else right()
        elif command == "two":
            second()
        else:
            fallback()
    except ValueError:
        recover()
        return 2
    except RuntimeError:
        return 3
    return 0
