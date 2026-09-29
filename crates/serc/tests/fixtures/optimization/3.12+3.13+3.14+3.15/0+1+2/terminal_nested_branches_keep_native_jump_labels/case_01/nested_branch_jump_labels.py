def dispatch(factories, records, available, text):
    words = text.split()
    if words == ['rules', 'show']:
        lines = []
        for key, entry in records.items():
            inputs_text = ','.join(entry['inputs'])
            options_text = ', '.join(f'{k}={v}' for k, v in entry['options'].items())
            range_text = f"range={entry['range']}" if 'range' in entry else ''
            lines.append(f"{key}: {entry['factory']}({options_text}): {inputs_text} {range_text}")
        return 'Rules:\n' + '\n'.join(lines)
    elif len(words) > 3 and words[:2] in (['rules', 'insert'], ['rules', 'update']):
        tokens = words[3:]
        options = {}
        if len([token for token in tokens if token.startswith('--transform')]) > 1:
            return 'Duplicate transform'
        transform, first, last = split_transform(tokens)
        if first >= 0 and transform is not None:
            options['transform'] = transform
        for index in range(len(tokens)):
            if first >= 0 and index >= first and index <= last:
                continue
            token = tokens[index]
            pieces = token.split('=')
            if len(pieces) != 2:
                return f'Bad token: {token}'
            options[pieces[0][2:]] = pieces[1]
        if 'factory' not in options.keys() or options['factory'] not in factories:
            return 'Missing factory'
        if 'inputs' not in options.keys() or len(options['inputs'].split(',')) == 0:
            return 'Missing inputs'
        factory_name = options.pop('factory')
        factory = factories[factory_name]
        inputs = options.pop('inputs').split(',')
        if len(inputs) > 1:
            if 'transform' not in options:
                return 'Missing transform'
            expression = options['transform']
            if valid_transform(expression):
                transform_fn = make_transform(expression)
            else:
                return 'Invalid transform'
            signature = inspect.signature(transform_fn)
            if len(signature.parameters) != len(inputs):
                return 'Wrong arity'
        needs_range = False
        for name in inputs:
            if '{index}' in name:
                needs_range = True
                break
        if needs_range:
            if 'range' not in options:
                return 'Missing range'
            try:
                bounds = parser.read(options['range'])
                if not isinstance(bounds, tuple) or len(bounds) != 3 or not all(isinstance(i, int) for i in bounds):
                    return 'Invalid range'
            except Exception:
                return 'Invalid range'
        else:
            if 'range' in options:
                return 'Unexpected range'
        for name in inputs:
            if '{index}' in name:
                for position in range(bounds[0], bounds[1], bounds[2]):
                    expanded = name.format(index=position)
                    if expanded not in available:
                        return f'Unknown input: {expanded}'
            else:
                if name not in available:
                    return f'Unknown input: {name}'
        schema = describe(factory)
        schema = {key: item for key, item in schema.items() if key != 'name' and key != 'inputs' and key != 'transform_fn'}
        requested = {key for key in options.keys() if key != 'transform' and key != 'range'}
        extra = sorted(set(requested) - set(schema.keys()))
        if extra:
            return f"Unknown options: {', '.join(extra)}"
        values: dict[str, int | float | str] = {}
        for item in schema.values():
            if item.default == MISSING:
                if item.name not in options:
                    return f'Missing option: {item.name}'
            if item.name in options:
                value = options[item.name]
                if item.kind in {int, float} and not value.replace('.', '', 1).isdigit():
                    return f'Invalid value: {item.name}; expected a number'
                if item.kind is int:
                    if value.isdigit():
                        values[item.name] = int(value)
                    else:
                        return f'Expected integer: {item.name}; got decimal input'
                elif item.kind is float:
                    values[item.name] = float(value)
                else:
                    assert False, 'Unexpected kind'
        record = {'factory': factory_name, 'options': values, 'inputs': inputs}
        if 'range' in options:
            record['range'] = parser.read(options['range'])
        if 'transform' in options:
            values['transform'] = options['transform']
        if words[1] == 'insert':
            records[words[2]] = record
            return f'Inserted {words[2]}'
        else:
            name = words[2]
            if name in records:
                records[name] = record
                return f'Updated {name}'
            else:
                return f'Unknown rule: {name}'
    elif len(words) == 3 and words[:2] == ['rules', 'delete']:
        name = words[2]
        if name in records:
            records.pop(name)
            return f'Deleted {name}'
        else:
            return f'Unknown rule: {name}'
    else:
        return None
