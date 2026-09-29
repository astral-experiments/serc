def nested_conditions(items, first, second, third):
    for item in items:
        try:
            if first and second:
                value = 1
                if second and third:
                    value = 2
                    if first and third:
                        value = operation()
        except Error:
            pass
