class Outer:
    def define():
        class Child[__T]:
            value = __T

        return Child


class ModuleLevel[__V]:
    value = __V
