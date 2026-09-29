"""Public source-shape proposals; not compiled or validated by this investigation."""


def nested_branch_return(outer, left, right, action):
    """Shape of corpus 178903: nested calls join before the normal try cleanup."""
    try:
        if outer:
            if left or right:
                action("first")
            else:
                action("second")
        else:
            if left or right:
                action("third")
            else:
                action("fourth")
    except Exception as error:
        action(error)
        raise
    return 0


def elif_branch_return(first, second, direct, action):
    """Shape of corpus 234559: two nested normal paths share a trailing return."""
    try:
        if first:
            result = action("prepare-first")
            if direct:
                action(result)
            else:
                action("write-first", result)
        elif second:
            result = action("prepare-second")
            if direct:
                action(result)
            else:
                action("write-second", result)
        else:
            action("finish")
    except ValueError as error:
        action(str(error))
    return 0


def plain_branch_small_return(condition, action):
    """Control: the original constant-return array is only three instructions."""
    if condition:
        action("first")
    else:
        action("second")
    return 0
