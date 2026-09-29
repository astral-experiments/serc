def f(items, flag, manager):
    for item in items:
        try:
            with manager:
                try:
                    work(item)
                    if flag:
                        break
                    continue
                finally:
                    cleanup_inner()
        finally:
            cleanup_outer()
