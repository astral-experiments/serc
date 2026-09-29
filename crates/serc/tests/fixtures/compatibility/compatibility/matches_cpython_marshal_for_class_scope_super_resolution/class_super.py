class Outer:
    def method(self):
        class Inner:
            super(Outer, self).method()

            def nested(inner_self):
                super(Outer, self).method()


class Annotated:
    def method(self):
        __class__: object
        super


class Shadowed:
    super = factory
    value = super(Shadowed, instance).method()
