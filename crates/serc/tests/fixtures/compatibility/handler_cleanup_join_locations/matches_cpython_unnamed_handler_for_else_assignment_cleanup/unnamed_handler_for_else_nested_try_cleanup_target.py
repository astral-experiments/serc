def decode(data, pieces, tag):
    try:
        value = data.decode("utf-8")
    except UnicodeDecodeError:
        for piece in pieces:
            try:
                value = piece.decode("utf-8")
                break
            except UnicodeDecodeError:
                continue
        else:
            value = None
    return tag, value
