def generator(flag, fd, acquire, release, close, timed_out, wait):
    def mark():
        remember(fd)

    acquired = False
    try:
        if flag:
            while True:
                try:
                    acquire(fd)
                    mark()
                    acquired = True
                    break
                except BlockingIOError:
                    if timed_out():
                        report(fd)
                        acquire(fd)
                        mark()
                        acquired = True
                        break
                    wait()
        else:
            while True:
                try:
                    acquire(fd)
                    mark()
                    acquired = True
                    break
                except OSError:
                    if timed_out():
                        report(fd)
                        acquire(fd)
                        mark()
                        acquired = True
                        break
                    wait()
        yield
    finally:
        try:
            if acquired:
                if flag:
                    release(fd)
                else:
                    release(fd)
        finally:
            close(fd)
