def collect(rows, output):
    for row in rows:
        if row.enabled:
            try:
                low = float(row.low)
                middle = float(row.middle)
                high = float(row.high)
            except ValueError:
                output.append("invalid input")
            else:
                if not all(is_finite(value) for value in (low, middle, high)):
                    output.append("nonfinite")
                elif not low <= middle <= high:
                    output.append("out of order")
            finish(row)
