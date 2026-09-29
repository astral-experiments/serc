def validate(items, upper, warnings):
    for field, value in items:
        if value is None:
            if field not in warnings:
                warnings.append(field)
        elif not finite(value) or not 0.0 <= value <= upper:
            warnings.append(field)
    return warnings
