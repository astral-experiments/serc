def test(items, limit, random_mode):
    count = 0
    success = False
    while count <= limit and not success:
        try:
            work()
            success = True
        except Error as error:
            count += 1
            if count > limit:
                raise RuntimeError() from error
            else:
                wait()
                if random_mode:
                    remove(items)
                    value = choose(items) if items else None
                else:
                    value = items.pop() if items else None
    done()
