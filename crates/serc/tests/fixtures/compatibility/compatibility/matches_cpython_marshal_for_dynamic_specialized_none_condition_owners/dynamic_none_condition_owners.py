class GeneratorPrint:
    def _types_to_string(self, value):
        return value

    def generate(self, func):
        log()
        log('Function', bold(func.name))
        with nested():
            desc = func.description
            if '\n' in desc:
                desc = desc[:desc.index('\n')]
            log('Description:', bold(desc))
            log('Return type:', bold(self._types_to_string(func.returns)))
            log('Pos args:', bold(str([x.name for x in func.posargs])))
            log('Opt args:', bold(str([x.name for x in func.optargs])))
            log('Varargs:', bold(func.varargs.name if func.varargs is not None else 'null'))
            log('Kwargs:', bold(str(list(func.kwargs.keys()))))


def verified_loop(matrix):
    norm = lambda values: root(sum(value**2 for value in values))
    previous = None
    precision = starting_precision(matrix)
    epsilon = 2**-precision
    while precision < maximum_precision:
        with working_precision(precision) as context:
            values = context.values(matrix)
            current = norm([part for value in values for part in split(value)])
            if previous is not None and difference(previous, current) < epsilon:
                return values
            previous = current
        precision *= 2
    raise PrecisionError


def clone_shape(checkout, bare, path):
    target = None
    try:
        if not bare:
            target = initialize(path)
            if checkout is None:
                checkout = True
        else:
            target = initialize_bare(path)
        consume(target, checkout)
    except Error:
        recover()


def repeated_bounds(bounds, values):
    try:
        first = transform(values)
        if bounds is not None:
            first = clip(first, bounds)
        second = transform(first)
        if bounds is not None:
            second = clip(second, bounds)
        third = transform(second)
        if bounds is not None:
            third = clip(third, bounds)
        return third
    except Error:
        recover()


def mapping_shape(manager, options, command):
    with manager:
        defaults = {'first': ['a'], 'second': ['b']}
        endings = {'first': [command], 'second': [command, 'tail']}
        if options is not None:
            command.extend(options)
        else:
            command.extend(defaults.get('first', []))
        command.extend(endings['first'])
