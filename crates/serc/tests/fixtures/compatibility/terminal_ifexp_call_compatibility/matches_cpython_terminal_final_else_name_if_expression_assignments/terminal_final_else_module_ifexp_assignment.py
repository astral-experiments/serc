if checking:
    class Selected(Base):
        pass
else:
    Selected = Fast if available else Slow
