class Echo:
    def __format__(self, spec):
        return spec

a, b, c = 1, 2, 3
formatted = (
    f"{a != b=:}",
    f"{a!=b=:}",
    f"{a != b != c=:}",
    f"{a != b and b != c=:}",
    f"{a if b != c else a=:}",
    f"{(a != b)=:}",
    f"{a != b=!s:}",
    f"{Echo():{a != b=:}}",
)
templates = (
    t"{a != b != c:}",
    t"{a != b != c=:}",
    t"{a != b and b != c:}",
    t"{a if b != c else a:}",
    t"{(a != b):}",
    t"{a != b!s:}",
    t"{a != b=}",
    t"""{(a # : fake ! conversion
):}""",
    t"""{(a # : fake ! conversion
)!s}""",
    t"""{(a # ! fake : conversion
)!r:>4}""",
    t"""{(a # ! fake : conversion
)}""",
)
