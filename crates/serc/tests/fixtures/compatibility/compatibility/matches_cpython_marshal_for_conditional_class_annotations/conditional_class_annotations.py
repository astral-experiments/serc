flag = True

class Conditional:
    direct: str
    if flag:
        enabled: int
    else:
        disabled: bytes

class WithClassCell:
    if flag:
        member: __class__

    def method(self):
        return __class__
