def decorate(cls):
    return cls


class Meta(type):
    pass


@decorate
class Example(object, metaclass=Meta):
    class_value = 1

    def __init__(self, value):
        self.value = value

    @property
    def doubled(self):
        return self.value * 2

    @classmethod
    def create(cls, value):
        return cls(value)

    @staticmethod
    def identity(value):
        return value
