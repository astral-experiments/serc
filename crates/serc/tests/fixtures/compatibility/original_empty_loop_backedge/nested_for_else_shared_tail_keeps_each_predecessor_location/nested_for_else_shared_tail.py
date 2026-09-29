def f(entries, patterns, classes):
    for name, obj in entries:
        if isfunction(obj):
            consume(name)
        elif isclass(obj):
            for pattern in patterns:
                if pattern.matches(name):
                    break
            else:
                attrs = attributes(obj)
                if attrs:
                    classes[name] = attrs
