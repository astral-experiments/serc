def run(a, b, c, fallback, output):
    if a < b < c: pass
    else:
        fallback()
    output.append(a)
    return output
