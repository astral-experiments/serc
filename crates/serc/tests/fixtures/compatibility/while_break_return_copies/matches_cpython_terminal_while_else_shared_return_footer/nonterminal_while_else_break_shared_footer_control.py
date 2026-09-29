def run(short, items, work):
    if short:
        work(items)
    else:
        j = 0
        while j < len(items):
            a, b = items[j]
            if a > 0 and b > 0:
                work(a)
                work(b)
                break
            j += 1
        else:
            if a > 0:
                work(a)
            if b > 0:
                work(b)
    work(items)
