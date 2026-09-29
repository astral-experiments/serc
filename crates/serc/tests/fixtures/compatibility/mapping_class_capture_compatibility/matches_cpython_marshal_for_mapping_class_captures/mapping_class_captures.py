def mapping_class(subject):
    match subject:
        case {"x": str() as value}:
            return value
        case _:
            return None


def mapping_class_guard(subject):
    match subject:
        case {"x": list() as values} if values:
            return values
        case _:
            return None


def nested_mapping_class(subject):
    match subject:
        case {"name": str() as name}:
            return name
        case {"id": int() as identifier}:
            return identifier
        case _:
            return None
