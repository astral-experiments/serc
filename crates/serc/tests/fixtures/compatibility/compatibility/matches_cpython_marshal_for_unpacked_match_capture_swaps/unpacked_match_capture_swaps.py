def nested_class(value):
    match value:
        case Attribute(value=Name(identifier), attribute=attribute):
            return identifier, attribute
        case _:
            return None


def paired_classes(value):
    match value:
        case (Left() as left, Right() as right):
            return left, right
        case _:
            return None
