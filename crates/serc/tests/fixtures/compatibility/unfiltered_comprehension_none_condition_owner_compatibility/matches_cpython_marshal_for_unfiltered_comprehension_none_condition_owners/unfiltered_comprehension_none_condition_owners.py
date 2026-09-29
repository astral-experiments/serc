def spark_names(names):
    return [name if name is None or len(name) > 1 else name[0] for name in names]


def vault_expirations(items, timezone):
    return [
        value if value is None or value.tzinfo is not None else value.replace(tzinfo=timezone)
        for value in (item.expiration for item in items)
    ]


def xarray_unfused(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array):
    return [
        value if replacement is None or array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]


def is_not_unfused(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array):
    return [
        value if replacement is not None or array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]


def chained_unfused(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, low, high):
    return [
        value if replacement is None or low < value < high else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]


def nested_call_unfused(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array, wrap):
    return [
        wrap(value if replacement is None or array.dtype.kind == "O" else replacement)
        for value, replacement in zip(values, replacements, strict=True)
    ]


def set_unfused(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array):
    return {
        value if replacement is None or array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
    }


def dict_value_unfused(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacements, array):
    return {
        value: value if replacement is None or array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
    }


def fused_control(values, replacements, array):
    return [
        value if replacement is None or array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]


def filtered_control(values, replacements, array, ready):
    return [
        value if replacement is None or array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
        if ready(value)
    ]


def call_control(values, replacements, array):
    return [
        value if replacement is None or array.accepts(value) else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]


def truth_control(values, replacements, array):
    return [
        value if replacement is None or array.ready else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]


def and_control(values, replacements, array):
    return [
        value if replacement is None and array.dtype.kind == "O" else replacement
        for value, replacement in zip(values, replacements, strict=True)
    ]
