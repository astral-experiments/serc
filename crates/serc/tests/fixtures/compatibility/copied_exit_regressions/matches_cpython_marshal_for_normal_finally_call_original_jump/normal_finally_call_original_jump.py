def normal_finally_target(work, condition, left, right, sink):
    try:
        work()
    finally:
        sink(left if condition else right)


def normal_finally_following_control(work, condition, left, right, sink, after):
    try:
        work()
    finally:
        sink(left if condition else right)
    after()
