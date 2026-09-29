import sys

def reimported():
    import sys
    sys.exit(1)

def shadowed(sys):
    sys.exit(1)

def only_local():
    import os
    os._exit(1)
