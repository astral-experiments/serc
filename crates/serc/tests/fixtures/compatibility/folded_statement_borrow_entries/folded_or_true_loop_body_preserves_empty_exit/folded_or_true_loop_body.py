def allocate(groups, total, tokens, all_full, consume):
    for group in groups:
        allocated = 0
        while allocated < total:
            full = choose()
            if full or all_full or True:
                count = tokens
            else:
                count = choose_count()
            count = min(total - allocated, count)
            consume(group, count)
            allocated += count
    return groups

def receive(total, tokens, consume):
    allocated = 0
    tensors = []
    while allocated < total:
        full = choose()
        if full or True:
            count = tokens
        else:
            count = choose_count()
        count = min(total - allocated, count)
        tensors.append((consume(count), count))
        allocated += count
    return tensors
