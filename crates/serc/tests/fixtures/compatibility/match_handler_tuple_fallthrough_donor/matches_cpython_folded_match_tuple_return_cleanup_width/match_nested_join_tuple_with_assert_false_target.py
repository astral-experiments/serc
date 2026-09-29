def choose(mode, first, second, values):
    match mode:
        case None:
            if first:
                if second:
                    left = [VALUE]
                    right = "first"
                elif values:
                    left = [format_value(second)]
                    right = "second"
                else:
                    left = [VALUE]
                    right = "third"
            elif second:
                left = [VALUE]
                right = "fourth"
            else:
                report(values)
                left = [VALUE]
                right = "fifth"
        case "pair":
            if first:
                left = [VALUE]
                right = "pair-first"
            else:
                left = [VALUE]
                right = "pair-second"
        case "restore":
            left = [format_value(item) for item in values]
            right = "restored"
        case "plain":
            left = [VALUE]
            right = "plain"
        case _:
            assert False, mode
    return left, right
