def membership_first(kind):
    if kind in {None, "sort"}:
        return 1
    return 0


def membership_second(kind):
    if kind in {None, "sort"}:
        return 2
    return 0


def nonstatic_membership_first(kind):
    if kind in {None, "linux"}:
        return 3
    return 0


def nonstatic_membership_second(kind):
    if kind in {None, "linux"}:
        return 4
    return 0


def shared_membership_first(kind):
    return kind in {None, "sort"}


def shared_membership_second(kind):
    return kind in {None, "sort"}


def direct_first():
    value = {"left", "center", "right"}
    return value, 1


def direct_second():
    value = {"left", "center", "right"}
    return value, 2


def direct_static_first():
    value = {"set", "no", "longer", "accepted"}
    return value, 1


def direct_static_second():
    value = {"set", "no", "longer", "accepted"}
    return value, 2


def identifier_seed():
    cubic = 1
    return cubic


def identifier_direct_first():
    value = {"cubic", "quintic"}
    return value, 1


def identifier_direct_second():
    value = {"cubic", "quintic"}
    return value, 2


def cached_character_first():
    value = {"a", "b", "c"}
    return value, 1


def cached_character_second():
    value = {"a", "b", "c"}
    return value, 2


def empty_string_first():
    value = {"", ".", "/"}
    return value, 1


def empty_string_second():
    value = {"", ".", "/"}
    return value, 2
