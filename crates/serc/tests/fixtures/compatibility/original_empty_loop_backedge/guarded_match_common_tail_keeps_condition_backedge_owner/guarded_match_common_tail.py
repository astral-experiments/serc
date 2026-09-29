def f(items, method_name, other_type):
    for entry in items:
        bindings = {}
        if callable := entry.methods.get(method_name):
            match callable.signature:
                case Signature((left, right)) if right.matches(bindings, other_type):
                    yield left.convert(bindings)
