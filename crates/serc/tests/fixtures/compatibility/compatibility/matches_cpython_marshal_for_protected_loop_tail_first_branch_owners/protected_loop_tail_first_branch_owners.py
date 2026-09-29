def with_owner(manager, items, match, accept, reject):
    with manager:
        for item in items:
            found = match(item)
            if found:
                if accept(found):
                    accept(item)
                else:
                    reject(found)
            else:
                reject(item)
