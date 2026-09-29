def porcupine_shaped(items, filepath):
    return consume(
        {
            name: filetype
            for name, filetype in items
            if any(
                fnmatch.fnmatch(filepath.as_posix(), '*/' + pattern)
                for pattern in filetype['filename_patterns']
            )
        },
        str(filepath),
    )


def all_porcupine_shaped(items, filepath):
    return consume(
        {
            name: filetype
            for name, filetype in items
            if all(
                fnmatch.fnmatch(filepath.as_posix(), '*/' + pattern)
                for pattern in filetype['filename_patterns']
            )
        },
        str(filepath),
    )


def outside_comprehension(values):
    return any(value for value in values)
