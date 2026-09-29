def f(outer, seed, use_pid, fast, p0, p1, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, left, right):
    if outer:
        if seed is None:
            left = plain(left)
        else:
            if use_pid:
                if fast:
                    left = quick(left, seed, pid=use_pid)
                else:
                    left = slow(left, seed, pid=use_pid)
            else:
                left = slow(left, seed)
    elif seed is None and ready(right):
        left = alternate(right)
        if use_pid:
            left = finish(left)
    else:
        left = fallback(right)
    return left
