class Base:
    pass

class Child(Base):
    def implicit(self, value):
        return super().method(
            value
        )

    def explicit(self, value):
        return super(Child, self).method(
            value
        )

    def keyword(self, value):
        return super().method(
            option=value,
        )

    def split_super(self, value):
        return super(
        ).method(
            value,
        )

    def multiline_attribute(self):
        return super().\
            value
