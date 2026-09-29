def run(items, manager, ready, choose, write, warn):
    for item in items:
        while ready(item):
            if choose(item):
                with manager(item) as resource:
                    write(resource)
                break
        else:
            warn(item)
