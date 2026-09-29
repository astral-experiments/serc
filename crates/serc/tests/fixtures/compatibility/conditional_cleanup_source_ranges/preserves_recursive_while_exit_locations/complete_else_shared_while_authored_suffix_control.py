def populate(selected, prepared, ready, pool, output, limit):
    if selected:
        if prepared:
            output.append(0)
        else:
            output.append(1)
            while len(output) < limit and pool:
                output.append(pool[len(output) % len(pool)])
            checkpoint(output)
    else:
        output.append(2)
    consume(output)
