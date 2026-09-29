sequence: [lambda: item for item in (1,)]
if True:
    mapping: {key: lambda: key for key in (2,)}
nested: [[lambda: (outer, inner) for inner in (3,)] for outer in (4,)]
