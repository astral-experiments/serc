def parse(source, decoder):
    parsed_values = []
    try:
        parsed_values.append(decode(source))
    except DecodeError:
        index = 0
        while True:
            index = source.find("{", index)
            if index == -1:
                break
            try:
                parsed, end = decoder.read(source, index)
            except DecodeError:
                index += 1
                continue
            parsed_values.append(parsed)
            index = end
    for parsed in parsed_values:
        result = convert(parsed)
        if result is not None:
            return result
    return None
