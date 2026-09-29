def final_else_target(branch, condition, left, right, sink):
    if branch:
        sink(left)
    else:
        sink(left if condition else right)


def tested_branch_after_region_target(work, branch, condition, left, right, sink):
    try:
        work()
    except Exception:
        pass
    if branch:
        sink(left if condition else right)


def generator_argument_target(instance, items):
    if items is not None:
        object.__setattr__(instance, "items", tuple(item for item in items))


def direct_call_control(condition, left, right, sink):
    sink(left if condition else right)
