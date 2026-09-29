def nested(groups):
    return (sum(item for item in group) for group in groups)

def lambda_inside(values):
    return ((lambda: value) for value in values)

result = consume(
    # leading
    item for item in values
    # trailing
)
