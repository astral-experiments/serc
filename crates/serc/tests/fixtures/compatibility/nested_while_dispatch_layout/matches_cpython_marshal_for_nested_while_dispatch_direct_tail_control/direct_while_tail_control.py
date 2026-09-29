def stream(queue, total):
    completed = 0
    next_index = 0
    pending = {}
    while completed < total:
        message = queue.get()
        kind = message[0]
        if kind == "result":
            _, index, metadata, rows = message
            completed += 1
            pending[int(index)] = rows
        elif kind == "error":
            raise RuntimeError(message) from None
        else:
            raise RuntimeError(kind)
        while next_index in pending:
            yield from pending.pop(next_index)
            next_index += 1
    finish()
