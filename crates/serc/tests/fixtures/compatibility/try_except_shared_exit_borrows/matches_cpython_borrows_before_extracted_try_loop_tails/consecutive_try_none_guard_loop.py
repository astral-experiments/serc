def probe(rows, output):
    total = 0
    for left, right in rows:
        try:
            if left is not None:
                total += int(float(left))
        except ValueError:
            pass
        try:
            if right is not None:
                output.append(float(right))
        except ValueError:
            pass
    return total
