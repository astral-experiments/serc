def comprehension_result(prefix, sizes, dimensions):
    return [prefix + tuple(sizes[dimension] for dimension in core_dimensions)
            for core_dimensions in dimensions]


def branch_result(results, shapes, dtypes):
    if results is None:
        arrays = tuple(make(shape, dtype)
                       for shape, dtype in zip(shapes, dtypes))
    else:
        arrays = tuple(make_like(result, shape, dtype)
                       for result, shape, dtype
                       in zip(results, shapes, dtypes))
    return arrays
