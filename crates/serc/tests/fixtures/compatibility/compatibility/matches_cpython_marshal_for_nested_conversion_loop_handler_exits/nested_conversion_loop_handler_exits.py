def classify(values):
    result = []
    for value in values:
        try:
            int(value)
        except (ValueError, TypeError):
            try:
                float(value)
            except (ValueError, TypeError):
                try:
                    complex(value)
                except (ValueError, TypeError):
                    result.append(type(value))
                else:
                    result.append(complex)
            else:
                result.append(float)
        else:
            result.append(int)
    return result


def two_levels(values):
    result = []
    for value in values:
        try:
            int(value)
        except (ValueError, TypeError):
            try:
                float(value)
            except (ValueError, TypeError):
                result.append(type(value))
            else:
                result.append(float)
        else:
            result.append(int)
    return result


def unlocated_bare_raise_cleanup(attribute, fields, options):
    try:
        assign(attribute)
    except Exception:
        fields = fields or {}
        options = options or {}
        if not (attribute in fields or attribute in options):
            raise
    finish()
