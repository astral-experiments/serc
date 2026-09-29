def run(item, manager, ready, choose, write, warn):
    while ready(item):
        if choose(item):
            with manager(item) as resource:
                write(resource)
            break
    else:
        warn(item)
