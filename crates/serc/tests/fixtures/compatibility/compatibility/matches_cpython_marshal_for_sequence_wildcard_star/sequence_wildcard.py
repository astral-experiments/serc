def prefix(value):
    match value:
        case [captured, *_]:
            return captured

def suffix(value):
    match value:
        case [*_, captured]:
            return captured

def both(value):
    match value:
        case [first, *_, last]:
            return first, last

def nested(value):
    match value:
        case [[captured], *_]:
            return captured
